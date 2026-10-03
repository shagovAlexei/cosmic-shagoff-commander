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
    let refuse = |h: &mut dyn Handler, why: &str| {
        let e = io::Error::new(io::ErrorKind::InvalidInput, why.to_string());
        Report {
            cancelled: h.error(archive, &e) == ErrorChoice::Cancel,
            completed: Vec::new(),
        }
    };
    let Some(format) = Format::detect(&name).filter(|f| f.is_tree()) else {
        return refuse(h, "not an archive that can be changed");
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
        return refuse(
            h,
            "file names in a legacy encoding; changing the archive would garble them",
        );
    }
    let dir = real.parent().unwrap_or(Path::new("."));
    let staging = match retry(h, archive, || archive::make_staging(dir)) {
        Ok(s) => s,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    let outcome = rewrite(&real, format, &name, &staging, change, h);
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
        give_up(
            h,
            archive,
            "some entries can't be unpacked; the archive is left as it was",
        )?;
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
        give_up(
            h,
            archive,
            "some entries can't be packed again; the archive is left as it was",
        )?;
    }
    retry(h, archive, || {
        let perms = fs::metadata(archive)?.permissions();
        fs::set_permissions(&new, perms)?;
        fs::rename(&new, archive)
    })?;
    Ok(done)
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
    let rel = archive::safe_path(rel).ok_or_else(|| bad("bad path inside the archive"))?;
    let parts: Vec<_> = rel.components().collect();
    let mut p = root.to_path_buf();
    for (i, c) in parts.iter().enumerate() {
        p.push(c);
        if (i + 1 < parts.len() || last)
            && fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(bad("the path goes through a link inside the archive"));
        }
    }
    Ok(p)
}

/// The change on the unpacked tree in `root`.
fn apply(
    archive: &Path,
    root: &Path,
    change: &Change,
    h: &mut dyn Handler,
) -> Result<Vec<PathBuf>, bool> {
    match change {
        Change::Add { sources, inner } => {
            let dir = retry(h, archive, || {
                let dir = if inner.as_os_str().is_empty() {
                    root.to_path_buf()
                } else {
                    inside(root, inner, true)?
                };
                fs::create_dir_all(&dir)?;
                Ok(dir)
            })?;
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
        Change::Rename { from, to } => {
            retry(h, &archive.join(to), || {
                ops::rename_noreplace(&inside(root, from, false)?, &inside(root, to, true)?)
            })?;
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
            Change::Rename {
                from: "ln".into(),
                to: "dl/x".into(),
            },
            Change::Add {
                sources: vec![edited.clone()],
                inner: "dl".into(),
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
