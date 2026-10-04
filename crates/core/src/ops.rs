//! File operations: copy / move / delete. Synchronous — run it on a worker thread. The UI is reached
//! only through `Handler`, so tests drive the engine with scripted answers.

use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Copy,
    Move,
    /// Ctrl+M: rename in place; never replaces, merges or falls back to copying.
    Rename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    Skip,
    ReplaceAll,
    SkipAll,
    ReplaceOlder,
    /// Write this one under the given name (same dir); asked again if that is taken too.
    Rename(String),
    /// Every conflict from now on: the first free `name (N).ext` (TC "keep both").
    RenameAll,
    Cancel,
}

/// First free `name (N).ext` next to `dst`: `a.txt` → `a (1).txt`; `a.tar.gz` → `a (1).tar.gz` (still
/// an archive); `.bashrc` → `.bashrc (1)`. Bytes, not UTF-8: a non-UTF-8 name keeps its bytes.
pub fn unique_name(dst: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let name = dst.file_name().unwrap_or_default().as_bytes();
    let mut cut = match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => i,
        _ => name.len(),
    };
    if name[..cut].ends_with(b".tar") && cut > 4 {
        cut -= 4;
    }
    let (stem, ext) = name.split_at(cut);
    (1..)
        .map(|n| {
            let mut new = stem.to_vec();
            new.extend_from_slice(format!(" ({n})").as_bytes());
            new.extend_from_slice(ext);
            dst.with_file_name(std::ffi::OsStr::from_bytes(&new))
        })
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("some number is free")
}

/// A name `Resolution::Rename` accepts: one path component.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
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
    /// Rename: empty, `.`, `..`, or contains `/`.
    BadName,
    /// Rename onto an existing entry where a dir is involved (would merge or clash).
    Exists(PathBuf),
}

/// Shift+F6 / F2: the single pair for renaming `src` to `name` in the same dir. Empty = unchanged.
/// A dir on either side of an existing name is refused (it would merge); file onto file goes on
/// to the engine's Replace/Skip question.
pub fn rename_pairs(src: &Path, name: &str) -> Result<Vec<(PathBuf, PathBuf)>, PlanError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(PlanError::BadName);
    }
    if src.file_name().is_some_and(|n| n == name) {
        return Ok(vec![]);
    }
    let dst = src.with_file_name(name);
    if let Ok(dm) = fs::symlink_metadata(&dst) {
        let src_is_dir = fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
        if src_is_dir || dm.is_dir() {
            return Err(PlanError::Exists(dst));
        }
    }
    Ok(vec![(src.to_path_buf(), dst)])
}

/// F8 (trash) / Shift+F8 (permanent, never following symlinks). Progress counts items.
pub fn delete(paths: &[PathBuf], permanent: bool, h: &mut dyn Handler) -> Report {
    let mut report = Report::default();
    let total = paths.len() as u64;
    for (i, p) in paths.iter().enumerate() {
        if h.cancelled() {
            report.cancelled = true;
            break;
        }
        loop {
            let r = if permanent {
                remove(p)
            } else {
                trash::delete(p).map_err(io::Error::other)
            };
            match r {
                Ok(()) => {
                    report.completed.push(p.clone());
                    break;
                }
                Err(e) => match h.error(p, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => break,
                    ErrorChoice::Cancel => {
                        report.cancelled = true;
                        return report;
                    }
                },
            }
        }
        h.progress(i as u64 + 1, total, p);
    }
    report
}

/// `remove_dir_all` does not follow symlinks (std ≥ 1.58); a link is removed as a file.
fn remove(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

const PART: &str = ".shagoff-part";

/// (source, destination) pairs, TC rules: existing dir → inside it; one source and no such path →
/// that name; several sources → inside `dest` (created on demand).
pub fn plan(sources: &[PathBuf], dest: &Path) -> Result<Vec<(PathBuf, PathBuf)>, PlanError> {
    if sources.is_empty() {
        return Err(PlanError::Empty);
    }
    // TC: a trailing `/` means "into this dir", even when it does not exist yet.
    let slash = dest.as_os_str().as_encoded_bytes().ends_with(b"/");
    let into_dir = slash || dest.is_dir() || sources.len() > 1;
    let mut pairs = Vec::with_capacity(sources.len());
    for src in sources {
        let dst = match (into_dir, src.file_name()) {
            (true, Some(name)) => dest.join(name),
            _ => dest.to_path_buf(),
        };
        let (s, d) = (resolve(src), resolve(&dst));
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
    /// Write to this path instead.
    Rename(PathBuf),
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
        let mut dst = dst.to_path_buf();
        if self.method == Method::Move
            && let Some(step) = self.rename(src, &mut dst, &meta)
        {
            return step;
        }
        let dst = dst.as_path();
        if self.method == Method::Rename {
            let size = tree_size(src);
            return match self.retry(src, || rename_noreplace(src, dst)) {
                Ok(()) => {
                    self.done += size;
                    self.h.progress(self.done, self.total, src);
                    Step::Done
                }
                Err(s) => s,
            };
        }
        if !meta.is_dir() && !meta.is_file() && !meta.file_type().is_symlink() {
            // A FIFO would block `open` forever and a device would stream endlessly.
            return self.refuse(src, 0, "special file (pipe, socket or device): not copied");
        }
        if meta.is_dir() {
            self.dir(src, dst, &meta)
        } else if meta.file_type().is_symlink() {
            self.symlink(src, dst, &meta)
        } else {
            self.file(src, dst, &meta)
        }
    }

    /// Move fast path. `None` = fall back to copy + delete (other device, or merging dirs) into
    /// `dst`, which a conflict answer may have changed to a new name.
    fn rename(&mut self, src: &Path, dst: &mut PathBuf, meta: &Metadata) -> Option<Step> {
        let mut replacing = false;
        if let Ok(dm) = fs::symlink_metadata(&*dst) {
            if meta.is_dir() || dm.is_dir() {
                return None;
            }
            match self.destination(src, meta, dst, meta.len()) {
                Ok(d) => {
                    replacing = d == *dst;
                    *dst = d;
                }
                Err(step) => return Some(step),
            }
        }
        let dst = dst.as_path();
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
        let created = match fs::symlink_metadata(dst) {
            Ok(dm) if dm.is_dir() => false, // merge into the existing dir
            Ok(_) => return self.clash(dst, tree_size(src)),
            Err(_) => {
                if let Err(s) = self.retry(dst, || fs::create_dir(dst)) {
                    return s;
                }
                true
            }
        };
        // Second line of defence behind `plan`: never walk into our own output.
        if let (Ok(s), Ok(d)) = (src.canonicalize(), dst.canonicalize())
            && d.starts_with(&s)
        {
            return self.refuse(dst, tree_size(src), "cannot copy a folder into itself");
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
        // After the children (a read-only dir would block them, writing them would bump the date);
        // only on dirs we made — merging into an existing dir must not change it.
        if created {
            if let (Ok(f), Ok(t)) = (File::open(dst), meta.modified()) {
                let _ = f.set_modified(t);
            }
            let _ = fs::set_permissions(dst, meta.permissions());
        }
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
        let dst = &match self.destination(src, meta, dst, 0) {
            Ok(d) => d,
            Err(step) => return step,
        };
        // Link at a fresh part name, then rename over: replacing stays atomic.
        let mut n = 0;
        let part = loop {
            let part = part_name(dst, n);
            match std::os::unix::fs::symlink(&target, &part) {
                Ok(()) => break part,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => match self.h.error(dst, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => return Step::Skipped,
                    ErrorChoice::Cancel => return Step::Cancel,
                },
            }
        };
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
        let dst = &match self.destination(src, meta, dst, size) {
            Ok(d) => d,
            Err(step) => return step,
        };
        let start = self.done;
        let part = loop {
            self.done = start;
            let result = create_part(dst).and_then(|(part, w)| {
                let done = self.copy_contents(src, w, meta);
                if !matches!(done, Ok(true)) {
                    let _ = fs::remove_file(&part); // only ever our own, freshly created part
                }
                done.map(|finished| finished.then_some(part))
            });
            match result {
                Ok(Some(part)) => break part,
                Ok(None) => return Step::Cancel,
                Err(e) => match self.h.error(src, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => {
                        self.done = start + size;
                        return Step::Skipped;
                    }
                    ErrorChoice::Cancel => return Step::Cancel,
                },
            }
        };
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

    /// Where to write: `dst` when free or approved for replacing, else what the conflict answer
    /// (or the sticky policy) says — asked again while a chosen new name is taken too.
    fn destination(
        &mut self,
        src: &Path,
        meta: &Metadata,
        dst: &Path,
        size: u64,
    ) -> Result<PathBuf, Step> {
        let mut dst = dst.to_path_buf();
        loop {
            let Ok(dm) = fs::symlink_metadata(&dst) else {
                return Ok(dst);
            };
            if dm.is_dir() {
                return Err(self.clash(&dst, size));
            }
            if self.approved.take_if(|p| *p == dst).is_some() {
                return Ok(dst);
            }
            match self.decide(src, meta, &dst, &dm) {
                Decision::Replace => return Ok(dst),
                Decision::Skip => {
                    self.done += size;
                    return Err(Step::Skipped);
                }
                Decision::Cancel => return Err(Step::Cancel),
                Decision::Rename(new) => dst = new,
            }
        }
    }

    /// Copy bytes, then mtime and permissions, into `part`. `Ok(false)` = cancelled.
    fn copy_contents(&mut self, src: &Path, mut w: File, meta: &Metadata) -> io::Result<bool> {
        let mut r = File::open(src)?;
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
        let answer = match &self.policy {
            Some(p) => p.clone(),
            None => {
                let r = self.h.conflict(&info(src, sm), &info(dst, dm));
                if matches!(
                    r,
                    Resolution::ReplaceAll
                        | Resolution::SkipAll
                        | Resolution::ReplaceOlder
                        | Resolution::RenameAll
                ) {
                    self.policy = Some(r.clone());
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
            // The dialog only sends valid names; a bad one from elsewhere must not escape the dir.
            Resolution::Rename(name) if valid_name(&name) => {
                Decision::Rename(dst.with_file_name(name))
            }
            Resolution::Rename(_) => Decision::Skip,
            Resolution::RenameAll => Decision::Rename(unique_name(dst)),
            Resolution::Cancel => Decision::Cancel,
        }
    }

    /// A file/link meets a directory of the same name (or the reverse).
    fn clash(&mut self, dst: &Path, size: u64) -> Step {
        self.refuse(dst, size, "an entry of another type has this name")
    }

    /// Report something we will not do; Retry acts as Skip (retrying cannot change it).
    fn refuse(&mut self, path: &Path, size: u64, why: &str) -> Step {
        let e = io::Error::other(why.to_string());
        match self.h.error(path, &e) {
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

/// `rename` that fails with `AlreadyExists` instead of replacing `dst`.
pub(crate) fn rename_noreplace(src: &Path, dst: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;
    match renameat_with(CWD, src, CWD, dst, RenameFlags::NOREPLACE) {
        // ponytail: filesystem without RENAME_NOREPLACE; check-then-rename leaves a tiny race window.
        Err(Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP) => {
            if fs::symlink_metadata(dst).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(src, dst)
        }
        r => r.map_err(io::Error::from),
    }
}

fn info(path: &Path, m: &Metadata) -> FileInfo {
    FileInfo {
        path: path.to_path_buf(),
        size: m.len(),
        mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    }
}

/// Hidden temp name next to `dst`: `.<name>.<pid>.<n>.shagoff-part`.
fn part_name(dst: &Path, n: u32) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(dst.file_name().unwrap_or_default());
    name.push(format!(".{}.{n}{PART}", std::process::id()));
    dst.with_file_name(name)
}

/// Create a part file that did not exist before (O_EXCL), so it can never be the source or a user file.
pub(crate) fn create_part(dst: &Path) -> io::Result<(PathBuf, File)> {
    let mut n = 0;
    loop {
        let part = part_name(dst, n);
        match File::options().write(true).create_new(true).open(&part) {
            Ok(f) => return Ok((part, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Where `p` really points, without resolving its own last component: `.`/`..` lexically, then the
/// longest existing ancestor of the parent canonicalized — a `..` or symlink detour cannot hide a path.
fn resolve(p: &Path) -> PathBuf {
    let mut norm = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                norm.pop();
            }
            other => norm.push(other),
        }
    }
    let (Some(parent), Some(name)) = (norm.parent(), norm.file_name()) else {
        return norm;
    };
    let mut missing = Vec::new();
    let mut cur = parent.to_path_buf();
    loop {
        if let Ok(mut out) = cur.canonicalize() {
            out.extend(missing.iter().rev());
            out.push(name);
            return out;
        }
        match cur.file_name() {
            Some(n) => {
                missing.push(n.to_os_string());
                cur.pop();
            }
            None => return norm,
        }
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

    fn no_part_files(dir: &Path) -> bool {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains("shagoff-part"))
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
    fn regression_trailing_slash_means_into_a_new_dir() {
        // TC: `newdir/` as the target of one file creates the dir and copies into it.
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        write(&a, "hello");
        let mut dest = d.path().join("newdir").into_os_string();
        dest.push("/");
        let r = copy(
            std::slice::from_ref(&a),
            Path::new(&dest),
            &mut Script::default(),
        );
        assert_eq!(r.completed, [a]);
        assert_eq!(read(&d.path().join("newdir/a")), "hello");
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
    fn unique_name_first_free_number() {
        let d = tempfile::tempdir().unwrap();
        let p = |n: &str| d.path().join(n);
        assert_eq!(unique_name(&p("a.txt")), p("a (1).txt"));
        assert_eq!(unique_name(&p("a.tar.gz")), p("a (1).tar.gz"));
        assert_eq!(unique_name(&p(".tar.gz")), p(".tar (1).gz"));
        {
            use std::os::unix::ffi::OsStrExt;
            let raw = d.path().join(std::ffi::OsStr::from_bytes(b"\xff.txt"));
            let want = d.path().join(std::ffi::OsStr::from_bytes(b"\xff (1).txt"));
            assert_eq!(unique_name(&raw), want);
        }
        assert_eq!(unique_name(&p("README")), p("README (1)"));
        assert_eq!(unique_name(&p(".bashrc")), p(".bashrc (1)"));
        write(&p("a (1).txt"), "");
        assert_eq!(unique_name(&p("a.txt")), p("a (2).txt"));
        assert!(valid_name("b.txt") && !valid_name("") && !valid_name("..") && !valid_name("x/y"));
    }

    #[test]
    fn conflict_rename_keeps_both_and_asks_again_when_taken() {
        let (d, h, r) = conflict_case(vec![Resolution::Rename("b".into())]);
        assert_eq!(read(&d.path().join("to/a")), "old");
        assert_eq!(read(&d.path().join("to/b")), "new");
        assert_eq!((h.asked, r.completed.len()), (1, 1));
        // the chosen name is taken too: asked again, then renamed once more
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("a"), "new");
        write(&d.path().join("to/a"), "old");
        write(&d.path().join("to/b"), "keep");
        let mut h = Script {
            conflicts: vec![
                Resolution::Rename("b".into()),
                Resolution::Rename("c".into()),
            ],
            ..Default::default()
        };
        copy(&[d.path().join("a")], &d.path().join("to"), &mut h);
        assert_eq!(h.asked, 2);
        assert_eq!(read(&d.path().join("to/b")), "keep");
        assert_eq!(read(&d.path().join("to/c")), "new");
        // a name that would leave the dir is never used
        let (d, _, r) = conflict_case(vec![Resolution::Rename("../x".into())]);
        assert!(!d.path().join("x").exists() && r.completed.is_empty());
    }

    #[test]
    fn rename_all_asks_once_and_numbers_each() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a.txt", "b.txt"] {
            write(&d.path().join(n), "new");
            write(&d.path().join("to").join(n), "old");
        }
        let mut h = Script {
            conflicts: vec![Resolution::RenameAll],
            ..Default::default()
        };
        copy(
            &[d.path().join("a.txt"), d.path().join("b.txt")],
            &d.path().join("to"),
            &mut h,
        );
        assert_eq!(h.asked, 1);
        assert_eq!(read(&d.path().join("to/a.txt")), "old");
        assert_eq!(read(&d.path().join("to/a (1).txt")), "new");
        assert_eq!(read(&d.path().join("to/b (1).txt")), "new");
    }

    #[test]
    fn move_with_rename_takes_the_new_name() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("a"), "new");
        write(&d.path().join("to/a"), "old");
        let mut h = Script {
            conflicts: vec![Resolution::Rename("b".into())],
            ..Default::default()
        };
        let pairs = plan(&[d.path().join("a")], &d.path().join("to")).unwrap();
        let r = transfer(Method::Move, &pairs, &mut h);
        assert_eq!(r.completed.len(), 1);
        assert!(!d.path().join("a").exists());
        assert_eq!(read(&d.path().join("to/a")), "old");
        assert_eq!(read(&d.path().join("to/b")), "new");
    }

    #[test]
    fn copied_dir_keeps_its_date() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("src/sub/f"), "x");
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for dir in ["src/sub", "src"] {
            File::open(d.path().join(dir))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        copy(
            &[d.path().join("src")],
            &d.path().join("dst"),
            &mut Script::default(),
        );
        for dir in ["dst", "dst/sub"] {
            let m = fs::metadata(d.path().join(dir))
                .unwrap()
                .modified()
                .unwrap();
            assert_eq!(m, t, "{dir}");
        }
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
        assert!(no_part_files(&d.path().join("to")));
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
        assert!(!to.join("bad").exists() && no_part_files(&to));
        assert_eq!(r.completed, [good]);
    }

    #[test]
    fn regression_rename_method_never_replaces_or_merges() {
        // Ctrl+M chain where an earlier step failed: the target is still a file of the batch.
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        write(&a, "a");
        write(&b, "b");
        write(&d.path().join("x/in"), "in");
        write(&d.path().join("y/keep"), "keep");
        let (x, y) = (d.path().join("x"), d.path().join("y"));
        let mut h = Script {
            errors: vec![ErrorChoice::Skip, ErrorChoice::Skip],
            ..Default::default()
        };
        let r = transfer(
            Method::Rename,
            &[(a.clone(), b.clone()), (x.clone(), y.clone())],
            &mut h,
        );
        assert_eq!(h.asked, 0, "never a Replace question");
        assert_eq!((read(&a), read(&b)), ("a".into(), "b".into()));
        assert!(
            x.join("in").exists() && !y.join("in").exists(),
            "dirs not merged"
        );
        assert_eq!(h.errored, [a, x]);
        assert!(r.completed.is_empty());
    }

    #[test]
    fn rename_method_renames_in_place() {
        let d = tempfile::tempdir().unwrap();
        let (a, c) = (d.path().join("a"), d.path().join("c"));
        write(&a, "a");
        let r = transfer(
            Method::Rename,
            &[(a.clone(), c.clone())],
            &mut Script::default(),
        );
        assert_eq!(read(&c), "a");
        assert_eq!(r.completed, [a]);
    }

    #[test]
    fn move_renames_on_same_fs() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        write(&a, "x");
        let to = d.path().join("to");
        fs::create_dir(&to).unwrap();
        let pairs = plan(std::slice::from_ref(&a), &to).unwrap();
        let r = transfer(Method::Move, &pairs, &mut Script::default());
        assert!(!a.exists());
        assert_eq!(read(&to.join("a")), "x");
        assert_eq!(r.completed, [a]);
    }

    #[test]
    fn move_merges_into_existing_dir() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("src/dir/new.txt"), "new");
        write(&d.path().join("to/dir/keep.txt"), "keep");
        let src = d.path().join("src/dir");
        let pairs = plan(std::slice::from_ref(&src), &d.path().join("to")).unwrap();
        transfer(Method::Move, &pairs, &mut Script::default());
        assert!(!src.exists());
        assert_eq!(read(&d.path().join("to/dir/new.txt")), "new");
        assert_eq!(read(&d.path().join("to/dir/keep.txt")), "keep");
    }

    #[test]
    fn move_keeps_skipped_files_in_source() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("src/dir/a"), "new-a");
        write(&d.path().join("src/dir/b"), "new-b");
        write(&d.path().join("to/dir/a"), "old-a");
        let src = d.path().join("src/dir");
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Default::default()
        };
        let pairs = plan(std::slice::from_ref(&src), &d.path().join("to")).unwrap();
        let r = transfer(Method::Move, &pairs, &mut h);
        assert_eq!(read(&src.join("a")), "new-a"); // skipped → still in source
        assert!(!src.join("b").exists()); // moved
        assert_eq!(read(&d.path().join("to/dir/b")), "new-b");
        assert_eq!(read(&d.path().join("to/dir/a")), "old-a");
        assert!(r.completed.is_empty()); // the dir was not fully moved
    }

    #[test]
    fn delete_permanent_does_not_follow_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        write(&outside.join("keep.txt"), "keep");
        let victim = d.path().join("victim");
        write(&victim.join("sub/x"), "x");
        symlink(&outside, victim.join("link")).unwrap();
        let file = d.path().join("f");
        write(&file, "f");
        let r = delete(
            &[victim.clone(), file.clone()],
            true,
            &mut Script::default(),
        );
        assert!(!victim.exists() && !file.exists());
        assert_eq!(read(&outside.join("keep.txt")), "keep");
        assert_eq!(r.completed, [victim, file]);
    }

    #[test]
    fn delete_error_skip_continues() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("missing");
        let file = d.path().join("f");
        write(&file, "f");
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = delete(&[missing.clone(), file.clone()], true, &mut h);
        assert_eq!(h.errored, [missing]);
        assert_eq!(r.completed, [file]);
    }

    #[test]
    fn regression_copy_never_truncates_its_own_source() {
        // Old scheme: part path of `foo` was `foo.shagoff-part` == the source → source truncated.
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("foo.shagoff-part");
        write(&src, "payload");
        let r = copy(
            std::slice::from_ref(&src),
            &d.path().join("foo"),
            &mut Script::default(),
        );
        assert_eq!(read(&src), "payload");
        assert_eq!(read(&d.path().join("foo")), "payload");
        assert_eq!(r.completed, [src]);
    }

    #[test]
    fn regression_user_file_named_like_a_part_is_untouched() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("a/foo"), "new");
        write(&d.path().join("b/foo.shagoff-part"), "mine");
        copy(
            &[d.path().join("a/foo")],
            &d.path().join("b"),
            &mut Script::default(),
        );
        assert_eq!(read(&d.path().join("b/foo")), "new");
        assert_eq!(read(&d.path().join("b/foo.shagoff-part")), "mine");
    }

    #[test]
    fn regression_sources_with_colliding_part_names_both_survive() {
        let d = tempfile::tempdir().unwrap();
        let (foo, part) = (d.path().join("s/foo"), d.path().join("s/foo.shagoff-part"));
        write(&foo, "one");
        write(&part, "two");
        let to = d.path().join("to");
        fs::create_dir(&to).unwrap();
        copy(&[foo, part], &to, &mut Script::default());
        assert_eq!(read(&to.join("foo")), "one");
        assert_eq!(read(&to.join("foo.shagoff-part")), "two");
    }

    #[test]
    fn regression_plan_sees_through_dotdot_and_symlink_detours() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        fs::create_dir(&a).unwrap();
        symlink(&a, d.path().join("alink")).unwrap();
        let via_dotdot = d.path().join("nope/../a/new");
        assert_eq!(
            plan(std::slice::from_ref(&a), &via_dotdot),
            Err(PlanError::IntoItself(a.clone()))
        );
        let via_link = d.path().join("alink/new/deeper");
        assert_eq!(
            plan(std::slice::from_ref(&a), &via_link),
            Err(PlanError::IntoItself(a))
        );
    }

    #[test]
    fn regression_special_files_are_skipped_not_hung() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("pipe");
        let ok = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !ok.is_ok_and(|s| s.success()) {
            return; // no mkfifo on this system
        }
        let to = d.path().join("to");
        fs::create_dir(&to).unwrap();
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = copy(std::slice::from_ref(&fifo), &to, &mut h);
        assert_eq!(h.errored, [fifo]);
        assert!(r.completed.is_empty());
        assert!(!to.join("pipe").exists());
    }

    #[test]
    fn regression_merge_keeps_existing_dir_mode() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("src/dir/x"), "x");
        fs::set_permissions(d.path().join("src/dir"), fs::Permissions::from_mode(0o700)).unwrap();
        write(&d.path().join("to/dir/keep"), "keep");
        fs::set_permissions(d.path().join("to/dir"), fs::Permissions::from_mode(0o755)).unwrap();
        copy(
            &[d.path().join("src/dir")],
            &d.path().join("to"),
            &mut Script::default(),
        );
        let mode = fs::metadata(d.path().join("to/dir"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn rename_pairs_validates_the_new_name() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, f) = (d.path().join("a"), d.path().join("b"), d.path().join("f"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        write(&f, "f");
        for bad in ["", ".", "..", "x/y", "/abs"] {
            assert_eq!(rename_pairs(&a, bad), Err(PlanError::BadName), "{bad:?}");
        }
        assert_eq!(rename_pairs(&a, "a"), Ok(vec![])); // unchanged → nothing to do
        assert_eq!(rename_pairs(&a, "b"), Err(PlanError::Exists(b.clone())));
        assert_eq!(rename_pairs(&f, "b"), Err(PlanError::Exists(b))); // file onto dir
        assert_eq!(rename_pairs(&a, "f"), Err(PlanError::Exists(f.clone()))); // dir onto file
        write(&d.path().join("g"), "g");
        // file onto file goes ahead: the engine asks Replace/Skip
        assert_eq!(
            rename_pairs(&f, "g"),
            Ok(vec![(f.clone(), d.path().join("g"))])
        );
        assert_eq!(rename_pairs(&f, "h"), Ok(vec![(f, d.path().join("h"))]));
    }
}
