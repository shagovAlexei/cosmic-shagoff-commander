//! Changing an archive (F5 / F6 / F7 / F8 / Shift+F6 / F4 inside one): unpack it next to itself,
//! change the files on disk, pack them again in the same format and rename over the original.
//! The original is untouched until the new one is complete, so cancel or an error leaves it as it
//! was. Every format works the same way and conflicts, errors and progress come from `unpack`,
//! `ops::transfer` and `pack`.
// ponytail: rewrites the whole archive; raw-copy zip entries if big archives get slow.

use crate::archive::{self, Format};
use crate::ops::{self, ErrorChoice, Handler, Method, Report};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// F5 / F6 into the archive: real files and dirs copied into the dir `inner`.
    Add {
        sources: Vec<PathBuf>,
        inner: PathBuf,
    },
    /// F8, or F6 out of the archive: entries (paths inside the archive).
    Delete(Vec<PathBuf>),
    /// F7: a new dir (path inside).
    Mkdir(PathBuf),
    /// Shift+F6: paths inside.
    Rename { from: PathBuf, to: PathBuf },
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
    let Some(format) = Format::detect(&name).filter(|f| f.is_tree()) else {
        let e = io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an archive that can be changed",
        );
        report.cancelled = h.error(archive, &e) == ErrorChoice::Cancel;
        return report;
    };
    let dir = archive.parent().unwrap_or(Path::new("."));
    let staging = match retry(h, archive, || archive::make_staging(dir)) {
        Ok(s) => s,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    let outcome = rewrite(archive, format, &name, &staging, change, h);
    let _ = fs::remove_dir_all(&staging); // never follows symlinks
    match outcome {
        Ok(done) => report.completed = done,
        Err(cancel) => report.cancelled = cancel,
    }
    report
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
        return Err(false);
    }
    let done = apply(archive, &root, change, h)?;
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
    let p = archive::pack(format, &root, &[(children.clone(), new.clone())], h);
    if p.cancelled {
        return Err(true);
    }
    if p.completed.len() != children.len() || !new.exists() {
        return Err(false);
    }
    retry(h, archive, || {
        let perms = fs::metadata(archive)?.permissions();
        fs::set_permissions(&new, perms)?;
        fs::rename(&new, archive)
    })?;
    Ok(done)
}

/// The change on the unpacked tree in `root`.
fn apply(
    archive: &Path,
    root: &Path,
    change: &Change,
    h: &mut dyn Handler,
) -> Result<Vec<PathBuf>, bool> {
    let inside = |p: &Path| archive::safe_path(p).map(|p| root.join(p));
    let bad = || io::Error::new(io::ErrorKind::InvalidInput, "bad path inside the archive");
    match change {
        Change::Add { sources, inner } => {
            let dir = if inner.as_os_str().is_empty() {
                root.to_path_buf()
            } else {
                inside(inner).ok_or(false)?
            };
            retry(h, archive, || fs::create_dir_all(&dir))?;
            let pairs: Vec<(PathBuf, PathBuf)> = sources
                .iter()
                .filter_map(|s| Some((s.clone(), dir.join(s.file_name()?))))
                .collect();
            let t = ops::transfer(Method::Copy, &pairs, h);
            if t.cancelled {
                return Err(true);
            }
            Ok(t.completed)
        }
        Change::Delete(entries) => {
            let mut done = Vec::new();
            for e in entries {
                let p = inside(e);
                retry(h, &archive.join(e), || {
                    let p = p.as_ref().ok_or_else(bad)?;
                    if fs::symlink_metadata(p)?.is_dir() {
                        fs::remove_dir_all(p)
                    } else {
                        fs::remove_file(p)
                    }
                })
                .map(|()| done.push(archive.join(e)))
                .or_else(|cancel| if cancel { Err(true) } else { Ok(()) })?;
            }
            Ok(done)
        }
        Change::Mkdir(d) => {
            let p = inside(d);
            retry(h, &archive.join(d), || {
                fs::create_dir(p.as_ref().ok_or_else(bad)?)
            })?;
            Ok(vec![archive.to_path_buf()])
        }
        Change::Rename { from, to } => {
            let (a, b) = (inside(from), inside(to));
            retry(h, &archive.join(to), || {
                let (a, b) = (a.as_ref().ok_or_else(bad)?, b.as_ref().ok_or_else(bad)?);
                ops::rename_noreplace(a, b)
            })?;
            Ok(vec![archive.to_path_buf()])
        }
        Change::Replace { entry, file } => {
            let p = inside(entry);
            retry(h, &archive.join(entry), || {
                fs::copy(file, p.as_ref().ok_or_else(bad)?).map(drop)
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
            };
            let r = modify(&a, &add(""), &mut Script::default());
            assert_eq!(r.completed, [new.clone()], "{f:?}");
            let r = modify(&a, &add("sub"), &mut Script::default());
            assert_eq!(r.completed, [new.clone()], "{f:?}");
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
    fn add_existing_name_asks() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(d.path(), Format::Zip);
        let src = d.path().join("a.txt");
        fs::write(&src, "new A").unwrap();
        let add = Change::Add {
            sources: vec![src],
            inner: "".into(),
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
        let ren = Change::Rename {
            from: "a.txt".into(),
            to: "sub/c.txt".into(),
        };
        modify(&a, &ren, &mut Script::default());
        let edited = d.path().join("edited");
        fs::write(&edited, "B2").unwrap();
        let rep = Change::Replace {
            entry: "sub/b.txt".into(),
            file: edited,
        };
        assert_eq!(
            modify(&a, &rep, &mut Script::default()).completed,
            [a.clone()]
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
        let ren = Change::Rename {
            from: "a.txt".into(),
            to: "sub/b.txt".into(),
        };
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
}
