//! File operations: copy / move / delete. Synchronous — run it on a worker thread. The UI is reached
//! only through `Handler`, so tests drive the engine with scripted answers.

use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    Skip,
    ReplaceAll,
    SkipAll,
    ReplaceOlder,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorChoice {
    Retry,
    Skip,
    Cancel,
}

/// What the conflict dialog shows about each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

pub trait Handler {
    /// Bytes for `transfer`, items for `delete`.
    fn progress(&mut self, done: u64, total: u64, current: &Path);
    fn conflict(&mut self, src: &FileInfo, dst: &FileInfo) -> Resolution;
    fn error(&mut self, path: &Path, err: &io::Error) -> ErrorChoice;
    fn cancelled(&self) -> bool;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub cancelled: bool,
    /// Sources processed completely (for unmarking).
    pub completed: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    Empty,
    IntoItself(PathBuf),
    SameFile(PathBuf),
}

const PART: &str = ".shagoff-part";

/// (source, destination) pairs, TC rules: existing dir → inside it; one source and no such path →
/// that name; several sources → inside `dest` (created on demand).
pub fn plan(sources: &[PathBuf], dest: &Path) -> Result<Vec<(PathBuf, PathBuf)>, PlanError> {
    if sources.is_empty() {
        return Err(PlanError::Empty);
    }
    let into_dir = dest.is_dir() || sources.len() > 1;
    let mut pairs = Vec::with_capacity(sources.len());
    for src in sources {
        let dst = match (into_dir, src.file_name()) {
            (true, Some(name)) => dest.join(name),
            _ => dest.to_path_buf(),
        };
        let (s, d) = (absolute(src), absolute(&dst));
        if s == d {
            return Err(PlanError::SameFile(src.clone()));
        }
        let src_is_dir = fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
        if src_is_dir && d.starts_with(&s) {
            return Err(PlanError::IntoItself(src.clone()));
        }
        pairs.push((src.clone(), dst));
    }
    Ok(pairs)
}

pub fn transfer(method: Method, pairs: &[(PathBuf, PathBuf)], h: &mut dyn Handler) -> Report {
    let total = pairs.iter().map(|(s, _)| tree_size(s)).sum();
    let mut t = Transfer {
        method,
        h,
        done: 0,
        total,
        policy: None,
        approved: None,
    };
    let mut report = Report::default();
    for (src, dst) in pairs {
        match t.entry(src, dst) {
            Step::Done => report.completed.push(src.clone()),
            Step::Skipped => {}
            Step::Cancel => {
                report.cancelled = true;
                break;
            }
        }
    }
    report
}

enum Step {
    Done,
    Skipped,
    Cancel,
}

enum Decision {
    Replace,
    Skip,
    Cancel,
}

struct Transfer<'a> {
    method: Method,
    h: &'a mut dyn Handler,
    done: u64,
    total: u64,
    /// Sticky answer from "… all" / "Replace older".
    policy: Option<Resolution>,
    /// Target already approved for replacing when a move falls back from rename to copy.
    approved: Option<PathBuf>,
}

impl Transfer<'_> {
    fn entry(&mut self, src: &Path, dst: &Path) -> Step {
        if self.h.cancelled() {
            return Step::Cancel;
        }
        let meta = match self.retry(src, || fs::symlink_metadata(src)) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if let Some(parent) = dst.parent()
            && let Err(s) = self.retry(parent, || fs::create_dir_all(parent))
        {
            return s;
        }
        if self.method == Method::Move
            && let Some(step) = self.rename(src, dst, &meta)
        {
            return step;
        }
        if meta.is_dir() {
            self.dir(src, dst, &meta)
        } else if meta.file_type().is_symlink() {
            self.symlink(src, dst, &meta)
        } else {
            self.file(src, dst, &meta)
        }
    }

    /// Move fast path. `None` = fall back to copy + delete (other device, or merging dirs).
    fn rename(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Option<Step> {
        let mut replacing = false;
        if let Ok(dm) = fs::symlink_metadata(dst) {
            if meta.is_dir() || dm.is_dir() {
                return None;
            }
            match self.decide(src, meta, dst, &dm) {
                Decision::Replace => replacing = true,
                Decision::Skip => {
                    self.done += meta.len();
                    return Some(Step::Skipped);
                }
                Decision::Cancel => return Some(Step::Cancel),
            }
        }
        let size = tree_size(src);
        loop {
            match fs::rename(src, dst) {
                Ok(()) => {
                    self.done += size;
                    self.h.progress(self.done, self.total, src);
                    return Some(Step::Done);
                }
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    if replacing {
                        self.approved = Some(dst.to_path_buf());
                    }
                    return None;
                }
                Err(e) => match self.h.error(src, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => {
                        self.done += size;
                        return Some(Step::Skipped);
                    }
                    ErrorChoice::Cancel => return Some(Step::Cancel),
                },
            }
        }
    }

    fn dir(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
        match fs::symlink_metadata(dst) {
            Ok(dm) if dm.is_dir() => {} // merge into the existing dir
            Ok(_) => return self.clash(dst, tree_size(src)),
            Err(_) => {
                if let Err(s) = self.retry(dst, || fs::create_dir(dst)) {
                    return s;
                }
            }
        }
        let names = match self.retry(src, || {
            fs::read_dir(src)?
                .map(|e| e.map(|e| e.file_name()))
                .collect::<io::Result<Vec<_>>>()
        }) {
            Ok(n) => n,
            Err(s) => return s,
        };
        let mut complete = true;
        for name in names {
            match self.entry(&src.join(&name), &dst.join(&name)) {
                Step::Done => {}
                Step::Skipped => complete = false,
                Step::Cancel => return Step::Cancel,
            }
        }
        // After the children: a read-only dir would have blocked writing them.
        let _ = fs::set_permissions(dst, meta.permissions());
        if !complete {
            return Step::Skipped;
        }
        if self.method == Method::Move
            && let Err(s) = self.retry(src, || fs::remove_dir(src))
        {
            return s;
        }
        Step::Done
    }

    fn symlink(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
        let target = match self.retry(src, || fs::read_link(src)) {
            Ok(t) => t,
            Err(s) => return s,
        };
        if let Some(step) = self.resolve_existing(src, meta, dst, 0) {
            return step;
        }
        // Link at the part path, then rename over: replacing stays atomic.
        let part = part_path(dst);
        let _ = fs::remove_file(&part);
        if let Err(s) = self.retry(dst, || std::os::unix::fs::symlink(&target, &part)) {
            return s;
        }
        if let Err(s) = self.retry(dst, || fs::rename(&part, dst)) {
            let _ = fs::remove_file(&part);
            return s;
        }
        if self.method == Method::Move
            && let Err(s) = self.retry(src, || fs::remove_file(src))
        {
            return s;
        }
        Step::Done
    }

    fn file(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
        let size = meta.len();
        if let Some(step) = self.resolve_existing(src, meta, dst, size) {
            return step;
        }
        let part = part_path(dst);
        let start = self.done;
        loop {
            self.done = start;
            match self.copy_contents(src, &part, meta) {
                Ok(true) => break,
                Ok(false) => {
                    let _ = fs::remove_file(&part);
                    return Step::Cancel;
                }
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    match self.h.error(src, &e) {
                        ErrorChoice::Retry => {}
                        ErrorChoice::Skip => {
                            self.done = start + size;
                            return Step::Skipped;
                        }
                        ErrorChoice::Cancel => return Step::Cancel,
                    }
                }
            }
        }
        if let Err(s) = self.retry(dst, || fs::rename(&part, dst)) {
            let _ = fs::remove_file(&part);
            return s;
        }
        self.h.progress(self.done, self.total, src);
        if self.method == Method::Move
            && let Err(s) = self.retry(src, || fs::remove_file(src))
        {
            return s;
        }
        Step::Done
    }

    /// If `dst` exists: ask (or apply the sticky policy). `None` = go ahead and replace.
    fn resolve_existing(
        &mut self,
        src: &Path,
        meta: &Metadata,
        dst: &Path,
        size: u64,
    ) -> Option<Step> {
        let dm = fs::symlink_metadata(dst).ok()?;
        if dm.is_dir() {
            return Some(self.clash(dst, size));
        }
        if self.approved.take_if(|p| p.as_path() == dst).is_some() {
            return None;
        }
        match self.decide(src, meta, dst, &dm) {
            Decision::Replace => None,
            Decision::Skip => {
                self.done += size;
                Some(Step::Skipped)
            }
            Decision::Cancel => Some(Step::Cancel),
        }
    }

    /// Copy bytes, then mtime and permissions, into `part`. `Ok(false)` = cancelled.
    fn copy_contents(&mut self, src: &Path, part: &Path, meta: &Metadata) -> io::Result<bool> {
        let mut r = File::open(src)?;
        let mut w = File::create(part)?;
        let mut buf = vec![0; 1 << 20];
        loop {
            if self.h.cancelled() {
                return Ok(false);
            }
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            w.write_all(&buf[..n])?;
            self.done += n as u64;
            self.h.progress(self.done, self.total, src);
        }
        w.set_modified(meta.modified()?)?;
        w.set_permissions(meta.permissions())?;
        Ok(true)
    }

    fn decide(&mut self, src: &Path, sm: &Metadata, dst: &Path, dm: &Metadata) -> Decision {
        let answer = match self.policy {
            Some(p) => p,
            None => {
                let r = self.h.conflict(&info(src, sm), &info(dst, dm));
                if matches!(
                    r,
                    Resolution::ReplaceAll | Resolution::SkipAll | Resolution::ReplaceOlder
                ) {
                    self.policy = Some(r);
                }
                r
            }
        };
        match answer {
            Resolution::Replace | Resolution::ReplaceAll => Decision::Replace,
            Resolution::Skip | Resolution::SkipAll => Decision::Skip,
            Resolution::ReplaceOlder if sm.modified().ok() > dm.modified().ok() => {
                Decision::Replace
            }
            Resolution::ReplaceOlder => Decision::Skip,
            Resolution::Cancel => Decision::Cancel,
        }
    }

    /// A file/link meets a directory of the same name (or the reverse): report it; Retry acts as Skip.
    fn clash(&mut self, dst: &Path, size: u64) -> Step {
        let e = io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an entry of another type has this name",
        );
        match self.h.error(dst, &e) {
            ErrorChoice::Cancel => Step::Cancel,
            _ => {
                self.done += size;
                Step::Skipped
            }
        }
    }

    fn retry<T>(&mut self, path: &Path, mut op: impl FnMut() -> io::Result<T>) -> Result<T, Step> {
        loop {
            match op() {
                Ok(v) => return Ok(v),
                Err(e) => match self.h.error(path, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => return Err(Step::Skipped),
                    ErrorChoice::Cancel => return Err(Step::Cancel),
                },
            }
        }
    }
}

fn info(path: &Path, m: &Metadata) -> FileInfo {
    FileInfo {
        path: path.to_path_buf(),
        size: m.len(),
        mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    }
}

fn part_path(dst: &Path) -> PathBuf {
    let mut name = dst.file_name().unwrap_or_default().to_os_string();
    name.push(PART);
    dst.with_file_name(name)
}

/// Canonical parent + own name: resolves `..` and symlinked parents, not the entry itself.
fn absolute(p: &Path) -> PathBuf {
    match (
        p.parent().and_then(|d| d.canonicalize().ok()),
        p.file_name(),
    ) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => p.to_path_buf(),
    }
}

/// Bytes under `p`, not following symlinks (for the progress bar).
fn tree_size(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    /// Scripted answers; records what the engine asked.
    #[derive(Default)]
    struct Script {
        conflicts: Vec<Resolution>,
        errors: Vec<ErrorChoice>,
        asked: usize,
        errored: Vec<PathBuf>,
        cancel_after_progress: bool,
        progressed: bool,
    }

    impl Handler for Script {
        fn progress(&mut self, _done: u64, _total: u64, _current: &Path) {
            self.progressed = true;
        }
        fn conflict(&mut self, _src: &FileInfo, _dst: &FileInfo) -> Resolution {
            self.asked += 1;
            if self.conflicts.is_empty() {
                Resolution::Cancel
            } else {
                self.conflicts.remove(0)
            }
        }
        fn error(&mut self, path: &Path, _err: &io::Error) -> ErrorChoice {
            self.errored.push(path.to_path_buf());
            if self.errors.is_empty() {
                ErrorChoice::Cancel
            } else {
                self.errors.remove(0)
            }
        }
        fn cancelled(&self) -> bool {
            self.cancel_after_progress && self.progressed
        }
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn copy(srcs: &[PathBuf], dest: &Path, h: &mut Script) -> Report {
        transfer(Method::Copy, &plan(srcs, dest).unwrap(), h)
    }

    fn set_mtime(p: &Path, t: SystemTime) {
        File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn plan_into_existing_dir_and_new_name() {
        let d = tempfile::tempdir().unwrap();
        let (a, to) = (d.path().join("a"), d.path().join("to"));
        write(&a, "x");
        fs::create_dir(&to).unwrap();
        assert_eq!(
            plan(std::slice::from_ref(&a), &to).unwrap(),
            [(a.clone(), to.join("a"))]
        );
        let new = d.path().join("b");
        assert_eq!(plan(std::slice::from_ref(&a), &new).unwrap(), [(a, new)]);
    }

    #[test]
    fn plan_many_into_missing_dir() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, to) = (d.path().join("a"), d.path().join("b"), d.path().join("new"));
        write(&a, "1");
        write(&b, "2");
        let pairs = plan(&[a.clone(), b.clone()], &to).unwrap();
        assert_eq!(pairs, [(a, to.join("a")), (b, to.join("b"))]);
    }

    #[test]
    fn plan_rejects_into_itself() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("dir");
        fs::create_dir_all(src.join("sub")).unwrap();
        assert_eq!(
            plan(std::slice::from_ref(&src), &src.join("sub")),
            Err(PlanError::IntoItself(src.clone()))
        );
        assert_eq!(
            plan(std::slice::from_ref(&src), &src),
            Err(PlanError::IntoItself(src))
        );
    }

    #[test]
    fn plan_rejects_same_file() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        write(&a, "x");
        assert_eq!(
            plan(std::slice::from_ref(&a), d.path()),
            Err(PlanError::SameFile(a.clone()))
        );
        assert_eq!(plan(&[], d.path()), Err(PlanError::Empty));
    }

    #[test]
    fn copy_file_keeps_content_mode_and_mtime() {
        let d = tempfile::tempdir().unwrap();
        let (a, to) = (d.path().join("a"), d.path().join("to"));
        write(&a, "hello");
        fs::set_permissions(&a, fs::Permissions::from_mode(0o640)).unwrap();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        set_mtime(&a, t);
        fs::create_dir(&to).unwrap();
        let r = copy(std::slice::from_ref(&a), &to, &mut Script::default());
        let c = to.join("a");
        assert_eq!(read(&c), "hello");
        assert_eq!(
            fs::metadata(&c).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::metadata(&c).unwrap().modified().unwrap(), t);
        assert_eq!(
            r,
            Report {
                cancelled: false,
                completed: vec![a.clone()]
            }
        );
        assert_eq!(read(&a), "hello"); // copy keeps the source
    }

    #[test]
    fn copy_tree_keeps_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        write(&src.join("x/deep.txt"), "deep");
        let outside = d.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, src.join("link")).unwrap();
        let to = d.path().join("to");
        fs::create_dir(&to).unwrap();
        copy(std::slice::from_ref(&src), &to, &mut Script::default());
        assert_eq!(read(&to.join("src/x/deep.txt")), "deep");
        let link = to.join("src/link");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), outside);
    }

    /// `to/a` exists with "old"; copy `a` ("new") over it with the scripted answers.
    fn conflict_case(answers: Vec<Resolution>) -> (tempfile::TempDir, Script, Report) {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("a"), "new");
        write(&d.path().join("to/a"), "old");
        let mut h = Script {
            conflicts: answers,
            ..Default::default()
        };
        let r = copy(&[d.path().join("a")], &d.path().join("to"), &mut h);
        (d, h, r)
    }

    #[test]
    fn conflict_replace_and_skip() {
        let (d, _, r) = conflict_case(vec![Resolution::Replace]);
        assert_eq!(read(&d.path().join("to/a")), "new");
        assert!(!r.cancelled);
        let (d, _, r) = conflict_case(vec![Resolution::Skip]);
        assert_eq!(read(&d.path().join("to/a")), "old");
        assert!(r.completed.is_empty());
    }

    #[test]
    fn conflict_cancel_stops() {
        let (d, _, r) = conflict_case(vec![Resolution::Cancel]);
        assert_eq!(read(&d.path().join("to/a")), "old");
        assert!(r.cancelled);
    }

    #[test]
    fn replace_all_and_skip_all_ask_once() {
        for (answer, expect) in [
            (Resolution::ReplaceAll, "new"),
            (Resolution::SkipAll, "old"),
        ] {
            let d = tempfile::tempdir().unwrap();
            for n in ["a", "b"] {
                write(&d.path().join(n), "new");
                write(&d.path().join("to").join(n), "old");
            }
            let mut h = Script {
                conflicts: vec![answer],
                ..Default::default()
            };
            copy(
                &[d.path().join("a"), d.path().join("b")],
                &d.path().join("to"),
                &mut h,
            );
            assert_eq!(h.asked, 1);
            assert_eq!(read(&d.path().join("to/a")), expect);
            assert_eq!(read(&d.path().join("to/b")), expect);
        }
    }

    #[test]
    fn replace_older_replaces_only_older_targets() {
        let d = tempfile::tempdir().unwrap();
        let old_t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let new_t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // a: source newer → replaced; b: source older → kept
        write(&d.path().join("a"), "src");
        set_mtime(&d.path().join("a"), new_t);
        write(&d.path().join("to/a"), "dst");
        set_mtime(&d.path().join("to/a"), old_t);
        write(&d.path().join("b"), "src");
        set_mtime(&d.path().join("b"), old_t);
        write(&d.path().join("to/b"), "dst");
        set_mtime(&d.path().join("to/b"), new_t);
        let mut h = Script {
            conflicts: vec![Resolution::ReplaceOlder],
            ..Default::default()
        };
        copy(
            &[d.path().join("a"), d.path().join("b")],
            &d.path().join("to"),
            &mut h,
        );
        assert_eq!(read(&d.path().join("to/a")), "src");
        assert_eq!(read(&d.path().join("to/b")), "dst");
        assert_eq!(h.asked, 1);
    }

    #[test]
    fn cancel_mid_file_keeps_existing_target() {
        let d = tempfile::tempdir().unwrap();
        let big = d.path().join("big");
        fs::write(&big, vec![7u8; 3 << 20]).unwrap(); // 3 chunks of 1 MiB
        write(&d.path().join("to/big"), "precious");
        let mut h = Script {
            conflicts: vec![Resolution::Replace],
            cancel_after_progress: true,
            ..Default::default()
        };
        let r = copy(&[big], &d.path().join("to"), &mut h);
        assert!(r.cancelled);
        assert_eq!(read(&d.path().join("to/big")), "precious");
        assert!(!d.path().join("to/big.shagoff-part").exists());
    }

    #[test]
    fn unreadable_file_skip_copies_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let (bad, good) = (d.path().join("bad"), d.path().join("good"));
        write(&bad, "secret");
        write(&good, "ok");
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&bad).is_ok() {
            return; // running as root: permissions don't apply
        }
        let to = d.path().join("to");
        fs::create_dir(&to).unwrap();
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = copy(&[bad.clone(), good.clone()], &to, &mut h);
        assert_eq!(h.errored, [bad]);
        assert_eq!(read(&to.join("good")), "ok");
        assert!(!to.join("bad").exists() && !to.join("bad.shagoff-part").exists());
        assert_eq!(r.completed, [good]);
    }
}
