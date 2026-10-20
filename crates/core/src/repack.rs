//! Changing an archive (F5 / F6 / F7 / F8 / Shift+F6 / F4 inside one): unpack it next to itself,
//! change the files on disk, pack them again in the same format and rename over the original.
//! The original is untouched until the new one is complete, so cancel or an error leaves it as it
//! was. Every format works the same way and conflicts, errors and progress come from `unpack`,
//! `ops::transfer` and `pack`. A zip (156) is faster: its entries are copied over raw, only
//! the change is written (`rewrite_zip`); where a question could come up it goes the whole way.

use crate::archive::{self, Format};
use crate::ops::{self, ErrorChoice, Handler, Method, Report};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// F5 / F6 into the archive: real files and dirs copied into the dir `inner`; a single source
    /// to an `inner` that is no dir there (and has no trailing `/`) gets that path as its name.
    Add {
        sources: Vec<PathBuf>,
        inner: PathBuf,
        /// F6: once the archive is rewritten, delete from the sources what went in (not what
        /// appeared or changed meanwhile; dirs only once empty).
        move_sources: bool,
    },
    /// F8, or F6 out of the archive: entries (paths inside the archive).
    Delete(Vec<PathBuf>),
    /// F7: a new dir (path inside).
    Mkdir(PathBuf),
    /// Shift+F6, Ctrl+M: (from, to) paths inside, done in this order (`multirename::plan`'s).
    Rename(Vec<(PathBuf, PathBuf)>),
    /// F4: the edited copy `file` replaces the entry `entry`.
    Replace { entry: PathBuf, file: PathBuf },
}

/// Rewrite `archive` with `change`. `completed`: Add — sources copied completely; Delete —
/// `archive/entry` of the deleted entries; the others — `[archive]`. Nothing completed: unchanged.
pub fn modify(archive: &Path, change: &Change, h: &mut dyn Handler) -> Report {
    let mut report = Report::default();
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let refuse = |h: &mut dyn Handler, why: &str| {
        let e = io::Error::new(io::ErrorKind::InvalidInput, why.to_string());
        Report {
            cancelled: h.error(archive, &e) == ErrorChoice::Cancel,
            completed: Vec::new(),
        }
    };
    let Some(format) = Format::detect(&name).filter(|f| f.is_tree()) else {
        return refuse(h, crate::why::NOT_CHANGEABLE);
    };
    // A symlinked archive: rewrite the real file, keep the link.
    let real = match retry(h, archive, || fs::canonicalize(archive)) {
        Ok(r) => r,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    if format == Format::Zip && legacy_zip_names(&real).unwrap_or(false) {
        // Read as CP437 and written back as UTF-8, they would turn into garbage for good.
        return refuse(h, crate::why::LEGACY_NAMES);
    }
    let dir = real.parent().unwrap_or(Path::new("."));
    let staging = match retry(h, archive, || archive::make_staging(dir)) {
        Ok(s) => s,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    let fast = (format == Format::Zip)
        .then(|| rewrite_zip(&real, &name, &staging, change, h))
        .flatten();
    let outcome = match fast {
        Some(outcome) => outcome,
        None => {
            // A try that gave up leaves its files behind: the whole way starts clean.
            let _ = fs::remove_dir_all(&staging);
            match retry(h, archive, || fs::create_dir(&staging)) {
                Ok(()) => rewrite(&real, format, &name, &staging, change, h),
                Err(cancel) => Err(cancel),
            }
        }
    };
    let _ = fs::remove_dir_all(&staging); // never follows symlinks
    match outcome {
        // Paths as the caller knows them (through the link, if any).
        Ok(done) => {
            report.completed = done
                .into_iter()
                .map(|p| match p.strip_prefix(&real) {
                    Ok(rest) if rest.as_os_str().is_empty() => archive.to_path_buf(),
                    Ok(rest) => archive.join(rest),
                    Err(_) => p,
                })
                .collect()
        }
        Err(cancel) => report.cancelled = cancel,
    }
    report
}

/// F5 / F6 / Ctrl+V from one archive into another, or into another dir of the same one: extract
/// `names` of the dir `inner` of `from` into a staging dir next to `to`, add them to its dir
/// `to_inner`, then with `move_after` delete from `from` what was added completely.
/// `completed`: `from/inner/name` of the names added.
// ponytail: up to two rewrites of a whole archive; fine until big archives get slow.
pub fn cross(
    from: &Path,
    inner: &Path,
    names: &[PathBuf],
    to: &Path,
    to_inner: &Path,
    move_after: bool,
    h: &mut dyn Handler,
) -> Report {
    let mut report = Report::default();
    // Next to the real file (as `modify` stages), not the link: one filesystem, one copy.
    let real = fs::canonicalize(to).unwrap_or_else(|_| to.to_path_buf());
    let dir = real.parent().unwrap_or(Path::new("."));
    let staging = match retry(h, to, || archive::make_staging(dir)) {
        Ok(s) => s,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    let out = archive::extract(from, inner, names, &staging, h);
    // Only what came out whole: a dir with a skipped entry must not be moved (deleted after).
    let sources: Vec<PathBuf> = (names.iter())
        .filter(|n| out.completed.contains(&from.join(inner).join(n)))
        .map(|n| staging.join(n))
        .collect();
    if !out.cancelled && !sources.is_empty() {
        let change = Change::Add {
            sources,
            inner: to_inner.to_path_buf(),
            move_sources: false,
        };
        let r = modify(to, &change, h);
        report.cancelled = r.cancelled;
        report.completed = (names.iter())
            .filter(|n| r.completed.contains(&staging.join(n)))
            .map(|n| from.join(inner).join(n))
            .collect();
    } else {
        report.cancelled = out.cancelled;
    }
    let _ = fs::remove_dir_all(&staging); // never follows symlinks
    if move_after && !report.cancelled && !report.completed.is_empty() {
        let entries = (report.completed.iter())
            .filter_map(|p| p.strip_prefix(from).ok().map(Path::to_path_buf))
            .collect();
        let d = modify(from, &Change::Delete(entries), h);
        report.cancelled = d.cancelled;
        report.completed = d.completed; // moved = deleted from the source too
    }
    report
}

/// A zip entry with a non-ASCII name not marked UTF-8 (Windows zips in cp866 and the like).
fn legacy_zip_names(archive: &Path) -> io::Result<bool> {
    let mut z = zip::ZipArchive::new(fs::File::open(archive)?)?;
    for i in 0..z.len() {
        let e = z.by_index_raw(i)?;
        let raw = e.name_raw();
        // The crate decodes unmarked names as CP437, so they differ from the raw bytes.
        if !raw.is_ascii() && std::str::from_utf8(raw) != Ok(e.name()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `Err(true)`: cancelled, `Err(false)`: given up (skipped); the archive is unchanged either way.
fn rewrite(
    archive: &Path,
    format: Format,
    name: &str,
    staging: &Path,
    change: &Change,
    h: &mut dyn Handler,
) -> Result<Vec<PathBuf>, bool> {
    let root = staging.join("t");
    retry(h, archive, || fs::create_dir(&root))?;
    let r = archive::unpack(&[archive.to_path_buf()], &root, false, h);
    if r.cancelled {
        return Err(true);
    }
    // Something was skipped: packing again would silently drop it.
    if r.completed.is_empty() {
        give_up(h, archive, crate::why::CANT_UNPACK)?;
    }
    let mut copies = Vec::new();
    let done = apply(
        archive,
        &root,
        change,
        &mut copies,
        &mut shown(h, &root, archive),
    )?;
    if done.is_empty() {
        return Err(false);
    }
    let mut children: Vec<PathBuf> = retry(h, archive, || {
        fs::read_dir(&root)?
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<_>>()
    })?;
    children.sort();
    let new = staging.join(name);
    // An encrypted archive stays encrypted, with the password it was opened with.
    let pw = archive::password(archive);
    let groups = [(children.clone(), new.clone())];
    let p = archive::pack_with(format, &root, &groups, pw.as_deref(), h);
    if p.cancelled {
        return Err(true);
    }
    if p.completed.len() != children.len() || !new.exists() {
        give_up(h, archive, crate::why::CANT_REPACK)?;
    }
    finish(archive, &new, change, done, &copies, h)
}

/// The new archive over the old one (its permissions kept); with F6 into it, the sources that
/// went in whole deleted.
fn finish(
    archive: &Path,
    new: &Path,
    change: &Change,
    done: Vec<PathBuf>,
    copies: &[(PathBuf, PathBuf)],
    h: &mut dyn Handler,
) -> Result<Vec<PathBuf>, bool> {
    retry(h, archive, || {
        let perms = fs::metadata(archive)?.permissions();
        fs::set_permissions(new, perms)?;
        fs::rename(new, archive)
    })?;
    if let Change::Add {
        move_sources: true, ..
    } = change
    {
        // Each file checked against its copy that was packed: same type, same size.
        let mut kept = |p: &Path, m: &fs::Metadata| {
            let copy = copies.iter().find_map(|(s, c)| {
                // Not `join("")`: that adds a `/`, and a file is no dir.
                let rest = p.strip_prefix(s).ok()?;
                Some(if rest.as_os_str().is_empty() {
                    c.clone()
                } else {
                    c.join(rest)
                })
            });
            copy.and_then(|c| fs::symlink_metadata(c).ok())
                .is_some_and(|c| c.file_type() == m.file_type() && c.len() == m.len())
        };
        // Never the archive itself (the app refuses that F6 too).
        let sources: Vec<PathBuf> = (done.iter())
            .filter(|s| !archive.starts_with(s))
            .cloned()
            .collect();
        ops::delete_kept(&sources, &mut kept, h);
    }
    Ok(done)
}

/// The names of a zip's entries; `None` when one is encrypted, a link or there twice — those go
/// the whole way (a password to write with, a link to keep a link, a name to keep apart).
fn zip_names(archive: &Path) -> io::Result<Option<Vec<String>>> {
    let mut z = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let mut names = Vec::with_capacity(z.len());
    let mut seen = std::collections::HashSet::new();
    for i in 0..z.len() {
        let e = z.by_index_raw(i)?;
        if e.encrypted() || e.is_symlink() || !seen.insert(e.name().to_string()) {
            return Ok(None);
        }
        names.push(e.name().to_string());
    }
    Ok(Some(names))
}

/// A zip name is `p` or under it (`p/`, `p/…`).
fn under(name: &str, p: &str) -> bool {
    name.strip_prefix(p)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// A zip, the fast way: its entries copied over raw (`archive::zip_rewrite`), only the change
/// written. `None`: not for this one — encrypted or link entries, a name already taken (a
/// conflict to ask about), an entry not there (an error to show): the whole rewrite does it.
fn rewrite_zip(
    archive: &Path,
    name: &str,
    staging: &Path,
    change: &Change,
    h: &mut dyn Handler,
) -> Option<Result<Vec<PathBuf>, bool>> {
    let names = zip_names(archive).ok()??;
    let s = |p: &Path| p.to_str().map(|s| s.trim_end_matches('/').to_string());
    let exists = |p: &str| exists_in(&names, p);
    let is_dir = |p: &str| p.is_empty() || names.iter().any(|n| n.starts_with(&format!("{p}/")));
    let root = staging.join("t");
    fs::create_dir(&root).ok()?;
    let mut keep: Keep = Box::new(|n| Some(n.to_string()));
    let mut copies = Vec::new();
    let done = match change {
        Change::Add { sources, inner, .. } => {
            let dir = s(inner)?;
            // As `apply`: one source to a path that is no dir there gets that name.
            let slash = inner.as_os_str().as_encoded_bytes().ends_with(b"/");
            let named = !slash && sources.len() == 1 && !is_dir(&dir);
            let taken = match named {
                true => exists(&dir),
                false => (sources.iter()).any(|src| {
                    src.file_name()
                        .and_then(|n| n.to_str())
                        .is_none_or(|n| exists(format!("{dir}/{n}").trim_start_matches('/')))
                }),
            };
            if taken {
                return None;
            }
            if !dir.is_empty() && is_dir(&dir) {
                fs::create_dir_all(inside(&root, Path::new(&dir), true).ok()?).ok()?;
            }
            apply(
                archive,
                &root,
                change,
                &mut copies,
                &mut shown(h, &root, archive),
            )
        }
        Change::Mkdir(d) => {
            if exists(&s(d)?) {
                return None;
            }
            apply(
                archive,
                &root,
                change,
                &mut copies,
                &mut shown(h, &root, archive),
            )
        }
        Change::Replace { entry, .. } => {
            let e = s(entry)?;
            if !names.contains(&e) {
                return None;
            }
            if let Some(parent) = entry.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(inside(&root, parent, true).ok()?).ok()?;
            }
            keep = Box::new(move |n| (n != e).then(|| n.to_string()));
            apply(
                archive,
                &root,
                change,
                &mut copies,
                &mut shown(h, &root, archive),
            )
        }
        Change::Delete(entries) => {
            let gone: Vec<String> = entries.iter().map(|e| s(e)).collect::<Option<_>>()?;
            if !gone.iter().all(|g| !g.is_empty() && exists(g)) {
                return None;
            }
            keep = Box::new(move |n| (!gone.iter().any(|g| under(n, g))).then(|| n.to_string()));
            Ok(entries.iter().map(|e| archive.join(e)).collect())
        }
        Change::Rename(pairs) => {
            let mut now = names.clone();
            for (from, to) in pairs {
                let (f, t) = (s(from)?, s(to)?);
                if f.is_empty() || t.is_empty() || !exists_in(&now, &f) || exists_in(&now, &t) {
                    return None;
                }
                for n in &mut now {
                    if let Some(rest) = n.strip_prefix(f.as_str()).filter(|_| under(n, &f)) {
                        *n = format!("{t}{rest}");
                    }
                }
            }
            let map: std::collections::HashMap<String, String> =
                names.iter().cloned().zip(now).collect();
            keep = Box::new(move |n| map.get(n).cloned());
            Ok(vec![archive.to_path_buf()])
        }
    };
    let done = match done {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => return Some(Err(false)),
        Err(cancel) => return Some(Err(cancel)),
    };
    let new = staging.join(name);
    let written = match archive::zip_rewrite(archive, &new, &*keep, &root, h) {
        Ok(Some(false)) => Ok(()),
        Ok(Some(true)) => give_up(h, archive, crate::why::CANT_REPACK),
        Ok(None) => Err(true),
        Err(e) => Err(h.error(archive, &e) == ErrorChoice::Cancel),
    };
    Some(written.and_then(|()| finish(archive, &new, change, done, &copies, h)))
}

/// A zip entry's new name, `None`: left out.
type Keep = Box<dyn Fn(&str) -> Option<String>>;

fn exists_in(names: &[String], p: &str) -> bool {
    names.iter().any(|n| under(n, p))
}

/// Questions and progress about the tree being changed name the archive's paths (`a.zip/sub/b`),
/// not the staging dir's.
struct Shown<'a> {
    h: &'a mut dyn Handler,
    root: &'a Path,
    archive: &'a Path,
}

fn shown<'a>(h: &'a mut dyn Handler, root: &'a Path, archive: &'a Path) -> Shown<'a> {
    Shown { h, root, archive }
}

impl Shown<'_> {
    fn map(&self, p: &Path) -> PathBuf {
        p.strip_prefix(self.root)
            .map_or_else(|_| p.to_path_buf(), |rel| self.archive.join(rel))
    }
}

impl Handler for Shown<'_> {
    fn progress(&mut self, done: u64, total: u64, current: &Path) {
        let current = self.map(current);
        self.h.progress(done, total, &current);
    }
    fn conflict(&mut self, src: &ops::FileInfo, dst: &ops::FileInfo) -> ops::Resolution {
        let at = |i: &ops::FileInfo| ops::FileInfo {
            path: self.map(&i.path),
            size: i.size,
            mtime: i.mtime,
        };
        let (src, dst) = (at(src), at(dst));
        self.h.conflict(&src, &dst)
    }
    fn error(&mut self, path: &Path, err: &io::Error) -> ErrorChoice {
        let path = self.map(path);
        self.h.error(&path, err)
    }
    fn cancelled(&self) -> bool {
        self.h.cancelled()
    }
}

/// Say why the archive stays unchanged; always `Err` (cancelled or given up).
fn give_up(h: &mut dyn Handler, archive: &Path, why: &str) -> Result<(), bool> {
    let e = io::Error::new(io::ErrorKind::InvalidData, why.to_string());
    Err(h.error(archive, &e) == ErrorChoice::Cancel)
}

/// `rel` inside `root`, refused when it climbs out or goes through a symlink of the unpacked
/// tree (a link entry would let the change write outside). `last`: the final component may not
/// be a link either.
fn inside(root: &Path, rel: &Path, last: bool) -> io::Result<PathBuf> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidInput, why.to_string());
    let rel = archive::safe_path(rel).ok_or_else(|| bad(crate::why::UNSAFE_PATH))?;
    let parts: Vec<_> = rel.components().collect();
    let mut p = root.to_path_buf();
    for (i, c) in parts.iter().enumerate() {
        p.push(c);
        if (i + 1 < parts.len() || last)
            && fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(bad(crate::why::THROUGH_LINK));
        }
    }
    Ok(p)
}

/// The change on the unpacked tree in `root`.
/// `copies`: Add — (source, its copy in `root`).
fn apply(
    archive: &Path,
    root: &Path,
    change: &Change,
    copies: &mut Vec<(PathBuf, PathBuf)>,
    h: &mut dyn Handler,
) -> Result<Vec<PathBuf>, bool> {
    match change {
        Change::Add { sources, inner, .. } => {
            // TC, as `ops::plan`: one source and no such dir (nor a trailing `/`) → that name.
            let slash = inner.as_os_str().as_encoded_bytes().ends_with(b"/");
            let named = !slash
                && sources.len() == 1
                && !inner.as_os_str().is_empty()
                && !inside(root, inner, true).is_ok_and(|p| p.is_dir());
            let (dir_in, name) = match (named, inner.parent(), inner.file_name()) {
                (true, Some(parent), Some(name)) => (parent, Some(name)),
                _ => (inner.as_path(), None),
            };
            let dir = retry(h, archive, || {
                let dir = if dir_in.as_os_str().is_empty() {
                    root.to_path_buf()
                } else {
                    inside(root, dir_in, true)?
                };
                fs::create_dir_all(&dir)?;
                Ok(dir)
            })?;
            let pairs: Vec<(PathBuf, PathBuf)> = sources
                .iter()
                .filter_map(|s| Some((s.clone(), dir.join(name.or(s.file_name())?))))
                .collect();
            let t = ops::transfer(Method::Copy, &pairs, h);
            copies.clone_from(&pairs);
            if t.cancelled {
                return Err(true);
            }
            Ok(t.completed)
        }
        Change::Delete(entries) => {
            let mut done = Vec::new();
            for e in entries {
                retry(h, &archive.join(e), || {
                    // The entry itself may be a link: removing it removes the link.
                    let p = inside(root, e, false)?;
                    if fs::symlink_metadata(&p)?.is_dir() {
                        fs::remove_dir_all(&p)
                    } else {
                        fs::remove_file(&p)
                    }
                })
                .map(|()| done.push(archive.join(e)))
                .or_else(|cancel| if cancel { Err(true) } else { Ok(()) })?;
            }
            Ok(done)
        }
        Change::Mkdir(d) => {
            // Nested names ("a/b") as F7 allows outside; an existing one is an error.
            retry(h, &archive.join(d), || {
                let p = inside(root, d, true)?;
                if fs::symlink_metadata(&p).is_ok() {
                    return Err(io::Error::from(io::ErrorKind::AlreadyExists));
                }
                fs::create_dir_all(&p)
            })?;
            Ok(vec![archive.to_path_buf()])
        }
        Change::Rename(pairs) => {
            // Skip skips that pair only (as Ctrl+M on disk); none done = the archive unchanged.
            let mut any = false;
            for (from, to) in pairs {
                retry(h, &archive.join(to), || {
                    ops::rename_noreplace(&inside(root, from, false)?, &inside(root, to, true)?)
                })
                .map(|()| any = true)
                .or_else(|cancel| if cancel { Err(true) } else { Ok(()) })?;
            }
            if !any {
                return Err(false);
            }
            Ok(vec![archive.to_path_buf()])
        }
        Change::Replace { entry, file } => {
            retry(h, &archive.join(entry), || {
                fs::copy(file, inside(root, entry, true)?).map(drop)
            })?;
            Ok(vec![archive.to_path_buf()])
        }
    }
}

/// Run `op` until it succeeds or the user skips (`Err(false)`) or cancels (`Err(true)`).
fn retry<T>(
    h: &mut dyn Handler,
    path: &Path,
    mut op: impl FnMut() -> io::Result<T>,
) -> Result<T, bool> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => match h.error(path, &e) {
                ErrorChoice::Retry => {}
                ErrorChoice::Skip => return Err(false),
                ErrorChoice::Cancel => return Err(true),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{FileInfo, Resolution};
    use std::os::unix::fs::PermissionsExt;

    #[derive(Default)]
    struct Script {
        conflicts: Vec<Resolution>,
        errors: usize,
        /// Answer errors with Cancel instead of Skip.
        cancel_errors: bool,
        conflicted: usize,
        /// Cancel after this many progress calls.
        cancel_after: Option<usize>,
        progress: usize,
    }
    impl Handler for Script {
        fn progress(&mut self, _: u64, _: u64, _: &Path) {
            self.progress += 1;
        }
        fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
            self.conflicted += 1;
            if self.conflicts.is_empty() {
                Resolution::Cancel
            } else {
                self.conflicts.remove(0)
            }
        }
        fn error(&mut self, _: &Path, _: &io::Error) -> ErrorChoice {
            self.errors += 1;
            if self.cancel_errors {
                ErrorChoice::Cancel
            } else {
                ErrorChoice::Skip
            }
        }
        fn cancelled(&self) -> bool {
            self.cancel_after.is_some_and(|n| self.progress >= n)
        }
    }

    /// `d/a.<ext>` holding `a.txt` ("A") and `sub/b.txt` ("B").
    fn sample(d: &Path, f: Format) -> PathBuf {
        let src = d.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "A").unwrap();
        fs::write(src.join("sub/b.txt"), "B").unwrap();
        let a = d.join(format!("a.{}", f.ext()));
        let r = archive::pack(
            f,
            &src,
            &[(vec![src.join("a.txt"), src.join("sub")], a.clone())],
            &mut Script::default(),
        );
        assert!(!r.cancelled && a.exists());
        fs::remove_dir_all(&src).unwrap();
        a
    }

    /// Every file in the archive as `path=content`, sorted.
    fn contents(a: &Path) -> Vec<String> {
        let out = tempfile::tempdir().unwrap();
        archive::unpack(
            &[a.to_path_buf()],
            out.path(),
            false,
            &mut Script::default(),
        );
        let mut v = Vec::new();
        let mut stack = vec![out.path().to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(out.path()).unwrap().display().to_string();
                if p.is_dir() {
                    v.push(format!("{rel}/"));
                    stack.push(p);
                } else {
                    v.push(format!("{rel}={}", fs::read_to_string(&p).unwrap()));
                }
            }
        }
        v.sort();
        v
    }

    #[test]
    fn regression_a_conflict_names_the_archive_not_the_staging_dir() {
        struct Asked(Vec<PathBuf>);
        impl Handler for Asked {
            fn progress(&mut self, _: u64, _: u64, _: &Path) {}
            fn conflict(&mut self, src: &FileInfo, dst: &FileInfo) -> Resolution {
                self.0.extend([src.path.clone(), dst.path.clone()]);
                Resolution::Skip
            }
            fn error(&mut self, _: &Path, _: &io::Error) -> ErrorChoice {
                ErrorChoice::Skip
            }
            fn cancelled(&self) -> bool {
                false
            }
        }
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let new = d.path().join("a.txt");
        fs::write(&new, "N").unwrap();
        let add = Change::Add {
            sources: vec![new.clone()],
            inner: PathBuf::new(),
            move_sources: false,
        };
        let mut h = Asked(Vec::new());
        let _ = modify(&a, &add, &mut h);
        assert_eq!(h.0, [new, a.join("a.txt")]);
    }

    /// Each entry's (name, compression method), in order.
    fn methods(a: &Path) -> Vec<(String, zip::CompressionMethod)> {
        let mut z = zip::ZipArchive::new(fs::File::open(a).unwrap()).unwrap();
        (0..z.len())
            .map(|i| {
                let e = z.by_index_raw(i).unwrap();
                (e.name().to_string(), e.compression())
            })
            .collect()
    }

    #[test]
    fn zip_entries_are_copied_over_raw_and_only_the_change_is_written() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&a).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.add_directory("docs/", stored).unwrap();
        w.start_file("docs/old.txt", stored).unwrap();
        io::Write::write_all(&mut w, b"OLD").unwrap();
        w.start_file("keep.txt", stored).unwrap();
        io::Write::write_all(&mut w, b"K").unwrap();
        w.finish().unwrap();
        let new = d.path().join("new.txt");
        fs::write(&new, "N").unwrap();
        let h = &mut Script::default();
        let add = Change::Add {
            sources: vec![new.clone()],
            inner: PathBuf::from("docs"),
            move_sources: false,
        };
        assert!(!modify(&a, &add, h).cancelled);
        let stored_m = zip::CompressionMethod::Stored;
        // The old entries as they were (Stored: the whole way packs everything Deflated), "docs/"
        // once, the new file in it.
        assert_eq!(
            methods(&a),
            [
                ("docs/".to_string(), stored_m),
                ("docs/old.txt".to_string(), stored_m),
                ("keep.txt".to_string(), stored_m),
                ("docs/new.txt".to_string(), zip::CompressionMethod::Deflated),
            ]
        );
        let rename = Change::Rename(vec![(PathBuf::from("docs"), PathBuf::from("papers"))]);
        assert!(!modify(&a, &rename, h).cancelled);
        let r = modify(
            &a,
            &Change::Delete(vec![PathBuf::from("papers/old.txt")]),
            h,
        );
        assert_eq!(r.completed, [a.join("papers/old.txt")]);
        assert!(!modify(&a, &Change::Mkdir(PathBuf::from("papers/x/y")), h).cancelled);
        assert_eq!(
            contents(&a),
            [
                "keep.txt=K",
                "papers/",
                "papers/new.txt=N",
                "papers/x/",
                "papers/x/y/"
            ]
        );
        assert_eq!(methods(&a)[1], ("keep.txt".to_string(), stored_m));
        assert_eq!(h.errors + h.conflicted, 0);
        // Taken already: the whole way, which asks (Cancel here: unchanged).
        let before = methods(&a);
        let add_taken = Change::Add {
            sources: vec![new],
            inner: PathBuf::from("papers"),
            move_sources: false,
        };
        assert!(modify(&a, &add_taken, h).cancelled);
        assert_eq!((h.conflicted, methods(&a)), (1, before));
    }

    #[test]
    fn cross_copies_between_archives_and_moves_inside_one() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let b = d.path().join("b.tar");
        fs::rename(sample(&d.path().join("t"), Format::Tar), &b).unwrap();
        let (n, h) = (vec![PathBuf::from("sub")], &mut Script::default());
        // zip → tar, into a dir of it
        let r = cross(&a, Path::new(""), &n, &b, Path::new("sub"), false, h);
        assert!(!r.cancelled && r.completed == [a.join("sub")]);
        assert_eq!(
            contents(&b),
            [
                "a.txt=A",
                "sub/",
                "sub/b.txt=B",
                "sub/sub/",
                "sub/sub/b.txt=B"
            ]
        );
        assert_eq!(contents(&a), ["a.txt=A", "sub/", "sub/b.txt=B"]);
        // F6 inside one archive: a.txt into sub/
        let n = vec![PathBuf::from("a.txt")];
        let r = cross(&a, Path::new(""), &n, &a, Path::new("sub"), true, h);
        assert!(!r.cancelled && r.completed == [a.join("a.txt")]);
        assert_eq!(contents(&a), ["sub/", "sub/a.txt=A", "sub/b.txt=B"]);
        assert!(no_leftovers(d.path()) && h.errors == 0);
    }

    #[test]
    fn cross_skipped_conflict_is_not_moved() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let b = d.path().join("b.tar");
        fs::rename(sample(&d.path().join("t"), Format::Tar), &b).unwrap();
        let h = &mut Script {
            conflicts: vec![Resolution::Skip],
            ..Script::default()
        };
        let n = vec![PathBuf::from("a.txt")];
        let r = cross(&a, Path::new(""), &n, &b, Path::new(""), true, h);
        assert!(r.completed.is_empty() && h.conflicted == 1);
        assert_eq!(contents(&a), ["a.txt=A", "sub/", "sub/b.txt=B"]);
    }

    #[test]
    fn regression_cross_move_keeps_a_dir_not_extracted_whole() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&a).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (n, body) in [("docs/x", "XXXXXXXXXXXX"), ("docs/y", "YYYYYYYYYYYY")] {
            z.start_file(n, stored).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        // docs/y's bytes no longer match its CRC: it fails to extract and is skipped.
        let mut bytes = fs::read(&a).unwrap();
        let at = bytes
            .windows(12)
            .position(|w| w == b"YYYYYYYYYYYY")
            .unwrap();
        bytes[at..at + 12].copy_from_slice(b"ZZZZZZZZZZZZ");
        fs::write(&a, bytes).unwrap();
        let b = sample(&d.path().join("t"), Format::Tar);
        let h = &mut Script::default();
        let n = vec![PathBuf::from("docs")];
        let r = cross(&a, Path::new(""), &n, &b, Path::new(""), true, h);
        assert!(h.errors > 0 && r.completed.is_empty());
        let mut z = zip::ZipArchive::new(fs::File::open(&a).unwrap()).unwrap();
        assert!(z.by_name("docs/y").is_ok()); // still in the source
    }

    #[test]
    fn rename_many_in_order_swaps_names() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        modify(
            &a,
            &Change::Rename(vec![("sub/b.txt".into(), "b.txt".into())]),
            &mut Script::default(),
        );
        // a ↔ b through a parked name, as `multirename::plan` orders it.
        let pairs = vec![
            ("a.txt".into(), ".p".into()),
            ("b.txt".into(), "a.txt".into()),
            (".p".into(), "b.txt".into()),
        ];
        let r = modify(&a, &Change::Rename(pairs), &mut Script::default());
        assert_eq!(r.completed, std::slice::from_ref(&a));
        assert_eq!(contents(&a), ["a.txt=B", "b.txt=A", "sub/"]);
    }

    #[test]
    fn rename_skip_skips_that_pair_only() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let h = &mut Script::default(); // errors answered Skip
        let pairs = vec![
            ("a.txt".into(), "sub".into()), // taken
            ("sub/b.txt".into(), "sub/c.txt".into()),
        ];
        let r = modify(&a, &Change::Rename(pairs), h);
        assert_eq!((h.errors, r.completed.len()), (1, 1));
        assert_eq!(contents(&a), ["a.txt=A", "sub/", "sub/c.txt=B"]);
        // Every pair skipped: nothing done, the archive left as it was.
        let before = fs::read(&a).unwrap();
        let r = modify(&a, &Change::Rename(vec![("a.txt".into(), "sub".into())]), h);
        assert!(r.completed.is_empty() && fs::read(&a).unwrap() == before);
    }

    fn no_leftovers(d: &Path) -> bool {
        fs::read_dir(d)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(".shagoff"))
    }

    #[test]
    fn add_into_root_and_subdir_every_format() {
        for f in [Format::Zip, Format::TarGz, Format::SevenZ, Format::Tar] {
            let d = tempfile::tempdir().unwrap();
            let a = sample(d.path(), f);
            let new = d.path().join("new.txt");
            fs::write(&new, "N").unwrap();
            let add = |inner: &str| Change::Add {
                sources: vec![new.clone()],
                inner: inner.into(),
                move_sources: false,
            };
            let r = modify(&a, &add(""), &mut Script::default());
            assert_eq!(r.completed, std::slice::from_ref(&new), "{f:?}");
            let r = modify(&a, &add("sub"), &mut Script::default());
            assert_eq!(r.completed, std::slice::from_ref(&new), "{f:?}");
            assert_eq!(
                contents(&a),
                [
                    "a.txt=A",
                    "new.txt=N",
                    "sub/",
                    "sub/b.txt=B",
                    "sub/new.txt=N"
                ],
                "{f:?}"
            );
            assert!(no_leftovers(d.path()), "{f:?}");
        }
    }

    #[test]
    fn regression_one_file_to_a_new_name_inside_is_a_file_not_a_dir() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let new = d.path().join("new.txt");
        fs::write(&new, "N").unwrap();
        let add = |inner: &str| Change::Add {
            sources: vec![new.clone()],
            inner: inner.into(),
            move_sources: false,
        };
        // TC, as `ops::plan`: one source, no such dir → that name; a trailing `/` → into a dir.
        let r = modify(&a, &add("sub/renamed.txt"), &mut Script::default());
        assert_eq!(r.completed, std::slice::from_ref(&new));
        modify(&a, &add("fresh/"), &mut Script::default());
        assert_eq!(
            contents(&a),
            [
                "a.txt=A",
                "fresh/",
                "fresh/new.txt=N",
                "sub/",
                "sub/b.txt=B",
                "sub/renamed.txt=N"
            ]
        );
    }

    /// Writes a new file into `dir` on every progress call (files appearing while it works).
    struct Busy {
        dir: PathBuf,
        n: usize,
    }
    impl Handler for Busy {
        fn progress(&mut self, _: u64, _: u64, _: &Path) {
            self.n += 1;
            let _ = fs::write(self.dir.join(format!("late{}", self.n)), "L");
        }
        fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
            Resolution::Cancel
        }
        fn error(&mut self, _: &Path, _: &io::Error) -> ErrorChoice {
            ErrorChoice::Skip
        }
        fn cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn regression_move_into_archive_keeps_files_that_appeared_meanwhile() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let src = d.path().join("dir");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("x.txt"), "X").unwrap();
        let add = Change::Add {
            sources: vec![src.clone()],
            inner: "".into(),
            move_sources: true,
        };
        let mut h = Busy {
            dir: src.clone(),
            n: 0,
        };
        let r = modify(&a, &add, &mut h);
        assert_eq!(r.completed, std::slice::from_ref(&src));
        let packed: Vec<String> = contents(&a);
        let left: Vec<String> = fs::read_dir(&src)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        // Whatever went in is gone from the dir; whatever is still there never went in.
        assert!(!src.join("x.txt").exists() && packed.contains(&"dir/x.txt=X".to_string()));
        assert!(!left.is_empty(), "files written while packing were deleted");
        for f in &left {
            assert!(
                !packed.iter().any(|p| p.starts_with(&format!("dir/{f}="))),
                "{f}"
            );
        }
    }

    #[test]
    fn add_existing_name_asks() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let src = d.path().join("a.txt");
        fs::write(&src, "new A").unwrap();
        let add = Change::Add {
            sources: vec![src],
            inner: "".into(),
            move_sources: false,
        };
        let mut h = Script {
            conflicts: vec![Resolution::Replace],
            ..Script::default()
        };
        modify(&a, &add, &mut h);
        assert_eq!(h.conflicted, 1);
        assert!(contents(&a).contains(&"a.txt=new A".to_string()));
        // Skipped: nothing to write, the archive is left alone (not even rewritten).
        let ino = |a: &Path| std::os::unix::fs::MetadataExt::ino(&fs::metadata(a).unwrap());
        let (before, inode) = (fs::read(&a).unwrap(), ino(&a));
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Script::default()
        };
        assert!(modify(&a, &add, &mut h).completed.is_empty());
        assert_eq!(fs::read(&a).unwrap(), before);
        assert_eq!(ino(&a), inode);
    }

    #[test]
    fn delete_skips_or_cancels_on_a_missing_entry() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let del = Change::Delete(vec!["missing".into(), "a.txt".into()]);
        let before = fs::read(&a).unwrap();
        let mut h = Script {
            cancel_errors: true,
            ..Script::default()
        };
        assert!(modify(&a, &del, &mut h).cancelled);
        assert_eq!(fs::read(&a).unwrap(), before);
        let r = modify(&a, &del, &mut Script::default());
        assert_eq!(r.completed, [a.join("a.txt")]);
        assert_eq!(contents(&a), ["sub/", "sub/b.txt=B"]);
    }

    #[test]
    fn delete_file_and_dir() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::TarGz);
        let r = modify(
            &a,
            &Change::Delete(vec!["sub".into()]),
            &mut Script::default(),
        );
        assert_eq!(r.completed, [a.join("sub")]);
        assert_eq!(contents(&a), ["a.txt=A"]);
        modify(
            &a,
            &Change::Delete(vec!["a.txt".into()]),
            &mut Script::default(),
        );
        assert!(contents(&a).is_empty()); // an empty archive, still readable
    }

    #[test]
    fn mkdir_rename_replace() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::SevenZ);
        modify(&a, &Change::Mkdir("sub/new".into()), &mut Script::default());
        let ren = Change::Rename(vec![("a.txt".into(), "sub/c.txt".into())]);
        modify(&a, &ren, &mut Script::default());
        let edited = d.path().join("edited");
        fs::write(&edited, "B2").unwrap();
        let rep = Change::Replace {
            entry: "sub/b.txt".into(),
            file: edited,
        };
        assert_eq!(
            modify(&a, &rep, &mut Script::default()).completed,
            std::slice::from_ref(&a)
        );
        assert_eq!(
            contents(&a),
            ["sub/", "sub/b.txt=B2", "sub/c.txt=A", "sub/new/"]
        );
    }

    #[test]
    fn existing_target_is_an_error_and_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let before = fs::read(&a).unwrap();
        let ren = Change::Rename(vec![("a.txt".into(), "sub/b.txt".into())]);
        let mut h = Script::default();
        assert!(modify(&a, &ren, &mut h).completed.is_empty());
        let mut h2 = Script::default();
        assert!(
            modify(&a, &Change::Mkdir("sub".into()), &mut h2)
                .completed
                .is_empty()
        );
        assert_eq!((h.errors, h2.errors), (1, 1));
        assert_eq!(fs::read(&a).unwrap(), before);
        assert!(no_leftovers(d.path()));
    }

    #[test]
    fn cancel_at_any_point_leaves_old_or_new_archive_and_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let new = d.path().join("new.txt");
        fs::write(&new, "N").unwrap();
        let add = Change::Add {
            sources: vec![new],
            inner: "".into(),
            move_sources: false,
        };
        let old = contents(&a);
        let saved = d.path().join("saved");
        fs::copy(&a, &saved).unwrap();
        let mut full = Script::default();
        modify(&a, &add, &mut full); // how many progress calls a whole run makes
        let changed = contents(&a);
        assert_ne!(old, changed);
        for n in 0..=full.progress {
            fs::copy(&saved, &a).unwrap();
            let mut h = Script {
                cancel_after: Some(n),
                ..Script::default()
            };
            let r = modify(&a, &add, &mut h);
            let now = contents(&a);
            assert!(now == old || now == changed, "after {n}: {now:?}");
            assert!(
                now == changed || r.cancelled,
                "after {n}: unchanged but not cancelled"
            );
            assert!(no_leftovers(d.path()), "after {n}");
        }
    }

    #[test]
    fn mkdir_nested() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let r = modify(&a, &Change::Mkdir("x/y".into()), &mut Script::default());
        assert_eq!(r.completed, std::slice::from_ref(&a));
        assert!(contents(&a).contains(&"x/y/".to_string()));
    }

    #[test]
    fn archive_permissions_kept() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        fs::set_permissions(&a, fs::Permissions::from_mode(0o600)).unwrap();
        modify(&a, &Change::Mkdir("x".into()), &mut Script::default());
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn unpackable_entries_keep_the_archive() {
        // An entry with `..` is never unpacked: rewriting would drop it.
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("evil.tar");
        let mut b = tar::Builder::new(fs::File::create(&a).unwrap());
        let mut hd = tar::Header::new_gnu();
        hd.set_size(1);
        hd.set_mode(0o644);
        hd.set_entry_type(tar::EntryType::Regular);
        hd.as_gnu_mut().unwrap().name[..7].copy_from_slice(b"../x.tx");
        hd.set_cksum();
        b.append(&hd, &b"x"[..]).unwrap();
        b.finish().unwrap();
        drop(b);
        let before = fs::read(&a).unwrap();
        let mut h = Script::default();
        assert!(
            modify(&a, &Change::Mkdir("d".into()), &mut h)
                .completed
                .is_empty()
        );
        assert_eq!(fs::read(&a).unwrap(), before);
    }

    #[test]
    fn regression_links_inside_never_lead_outside() {
        let d = tempfile::tempdir().unwrap();
        let victim = d.path().join("victim.txt");
        fs::write(&victim, "precious").unwrap();
        let outside = d.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let src = d.path().join("src");
        fs::create_dir(&src).unwrap();
        std::os::unix::fs::symlink(&victim, src.join("ln")).unwrap();
        std::os::unix::fs::symlink(&outside, src.join("dl")).unwrap();
        let a = d.path().join("l.tar");
        let r = archive::pack(
            Format::Tar,
            &src,
            &[(vec![src.join("ln"), src.join("dl")], a.clone())],
            &mut Script::default(),
        );
        assert!(!r.cancelled);
        let edited = d.path().join("edited");
        fs::write(&edited, "evil").unwrap();
        let changes = [
            Change::Replace {
                entry: "ln".into(),
                file: edited.clone(),
            },
            Change::Mkdir("dl/new".into()),
            Change::Rename(vec![("ln".into(), "dl/x".into())]),
            Change::Add {
                sources: vec![edited.clone()],
                inner: "dl".into(),
                move_sources: false,
            },
            Change::Delete(vec!["dl/anything".into()]),
        ];
        for c in changes {
            let before = fs::read(&a).unwrap();
            let r = modify(&a, &c, &mut Script::default());
            assert!(r.completed.is_empty(), "{c:?}");
            assert_eq!(fs::read(&a).unwrap(), before, "{c:?}");
        }
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[test]
    fn regression_symlinked_archive_stays_a_link() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let link = d.path().join("link.zip");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        let r = modify(&link, &Change::Mkdir("x".into()), &mut Script::default());
        assert_eq!(r.completed, std::slice::from_ref(&link));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(contents(&a).contains(&"x/".to_string()));
    }

    #[test]
    fn regression_zip_names_in_a_legacy_encoding_are_kept() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("dos.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&a).unwrap());
        z.start_file("aXain.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        // cp866 "п" in place of X, without the UTF-8 flag (as Windows zips have it).
        let bytes = fs::read(&a).unwrap();
        let patched: Vec<u8> = bytes
            .windows(9)
            .enumerate()
            .fold(bytes.clone(), |mut v, (i, w)| {
                if w == b"aXain.txt" {
                    v[i + 1] = 0xAF;
                }
                v
            });
        fs::write(&a, &patched).unwrap();
        let mut h = Script::default();
        assert!(
            modify(&a, &Change::Mkdir("x".into()), &mut h)
                .completed
                .is_empty()
        );
        assert_eq!(h.errors, 1); // told why
        assert_eq!(fs::read(&a).unwrap(), patched);
    }
}
