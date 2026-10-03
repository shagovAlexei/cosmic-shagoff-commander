use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{
    ffi::OsString,
    fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Display name (lossy UTF-8).
    pub name: String,
    /// Real on-disk name; use it for paths, never `name`.
    pub os_name: OsString,
    /// Extension of files only (`archive.tar.gz` → `gz`, `.bashrc` → empty); always empty for dirs.
    pub ext: String,
    pub size: u64,
    pub mtime: SystemTime,
    pub kind: Kind,
    pub is_link: bool,
    /// Unix permission bits (of the link target for symlinks).
    pub mode: u32,
    /// (uid, gid) of the entry itself; `None` inside archives or when stat failed.
    pub owner: Option<(u32, u32)>,
    /// Where a symlink points (as written in the link).
    pub target: Option<std::path::PathBuf>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
}

/// Lists `path` without `..`. Unreadable items are skipped; only failing to read the dir itself is an error.
pub fn scan(path: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let dir = match fs::read_dir(path) {
        Ok(d) => d,
        // "/x/a.zip/docs": a path through an archive file is listed from the archive.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotADirectory | io::ErrorKind::NotFound
            ) =>
        {
            return match crate::archive::split_path(path) {
                Some((archive, inner)) => crate::archive::list(&archive, &inner, show_hidden),
                None => Err(e),
            };
        }
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for item in dir {
        let Ok(item) = item else { continue };
        let os_name = item.file_name();
        let name = os_name.to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        // DirEntry::metadata does not follow symlinks.
        let Ok(lmeta) = item.metadata() else {
            // Dir readable but not searchable (r-- without x): stat fails, readdir's type still works.
            let Ok(ft) = item.file_type() else { continue };
            let kind = if ft.is_dir() { Kind::Dir } else { Kind::File };
            out.push(Entry {
                ext: if kind == Kind::File {
                    ext_of(&name)
                } else {
                    String::new()
                },
                os_name,
                name,
                size: 0,
                mtime: UNIX_EPOCH,
                kind,
                is_link: ft.is_symlink(),
                mode: 0,
                owner: None,
                target: None,
            });
            continue;
        };
        let is_link = lmeta.file_type().is_symlink();
        let target = if is_link {
            fs::metadata(item.path()).ok()
        } else {
            Some(lmeta.clone())
        };
        let (kind, size, meta) = match &target {
            Some(m) if m.is_dir() => (Kind::Dir, 0, m),
            Some(m) => (Kind::File, m.len(), m),
            None => (Kind::File, 0, &lmeta), // broken symlink
        };
        let ext = if kind == Kind::File {
            ext_of(&name)
        } else {
            String::new()
        };
        out.push(Entry {
            os_name,
            mtime: meta.modified().unwrap_or(UNIX_EPOCH),
            name,
            ext,
            size,
            kind,
            is_link,
            mode: meta.permissions().mode(),
            owner: Some((lmeta.uid(), lmeta.gid())),
            target: is_link.then(|| fs::read_link(item.path()).ok()).flatten(),
        });
    }
    Ok(out)
}

pub(crate) fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn find<'a>(v: &'a [Entry], name: &str) -> &'a Entry {
        v.iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} not listed"))
    }

    #[test]
    fn regression_dir_readable_but_not_searchable_lists_names() {
        // r-- without x: names are readable, stat on children fails.
        let d = tempfile::tempdir().unwrap();
        let locked = d.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("f.txt"), "x").unwrap();
        fs::create_dir(locked.join("sub")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o444)).unwrap();
        let v = scan(&locked, false);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let v = v.unwrap();
        assert_eq!(find(&v, "f.txt").kind, Kind::File);
        assert_eq!(find(&v, "sub").kind, Kind::Dir);
    }

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "hello").unwrap();
        fs::write(d.path().join("archive.tar.gz"), "").unwrap();
        fs::write(d.path().join(".bashrc"), "").unwrap();
        fs::write(d.path().join("noext"), "").unwrap();
        fs::create_dir(d.path().join("sub.d")).unwrap();
        symlink(d.path().join("sub.d"), d.path().join("link_dir")).unwrap();
        symlink(d.path().join("nope"), d.path().join("broken")).unwrap();
        d
    }

    #[test]
    fn scan_lists_files_dirs_and_links() {
        let d = fixture();
        let v = scan(d.path(), true).unwrap();
        assert_eq!(v.len(), 7);
        let a = find(&v, "a.txt");
        assert_eq!(
            (a.kind, a.size, a.ext.as_str(), a.is_link),
            (Kind::File, 5, "txt", false)
        );
        let sub = find(&v, "sub.d");
        assert_eq!((sub.kind, sub.ext.as_str()), (Kind::Dir, ""));
        let link = find(&v, "link_dir");
        assert_eq!((link.kind, link.is_link), (Kind::Dir, true));
        let broken = find(&v, "broken");
        assert_eq!(
            (broken.kind, broken.size, broken.is_link),
            (Kind::File, 0, true)
        );
    }

    #[test]
    fn scan_ext_rules() {
        let d = fixture();
        let v = scan(d.path(), true).unwrap();
        assert_eq!(find(&v, "archive.tar.gz").ext, "gz");
        assert_eq!(find(&v, ".bashrc").ext, "");
        assert_eq!(find(&v, "noext").ext, "");
    }

    #[test]
    fn scan_hides_dotfiles_unless_asked() {
        let d = fixture();
        let v = scan(d.path(), false).unwrap();
        assert!(v.iter().all(|e| !e.name.starts_with('.')));
        assert_eq!(v.len(), 6);
    }

    #[test]
    fn scan_reads_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x");
        fs::write(&p, "").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
        let v = scan(d.path(), true).unwrap();
        assert_eq!(v[0].mode & 0o777, 0o640);
    }

    #[test]
    fn scan_missing_dir_is_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(scan(&d.path().join("missing"), true).is_err());
    }

    #[test]
    fn scan_non_utf8_name_is_listed() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(OsStr::from_bytes(b"bad\xffname")), "").unwrap();
        let v = scan(d.path(), true).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].name.starts_with("bad"));
        assert_eq!(v[0].os_name, OsStr::from_bytes(b"bad\xffname"));
    }

    #[test]
    fn scan_inside_archive() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&a).unwrap());
        z.start_file("d/f.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        let v = scan(&a.join("d"), false).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "f.txt");
        assert!(scan(&d.path().join("missing"), false).is_err());
    }

    #[test]
    fn scan_reads_owner_and_link_target() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f"), "x").unwrap();
        std::os::unix::fs::symlink("f", d.path().join("l")).unwrap();
        let e = scan(d.path(), false).unwrap();
        let get = |n: &str| e.iter().find(|e| e.name == n).unwrap().clone();
        let me = std::fs::metadata(d.path()).unwrap();
        assert_eq!(get("f").owner, Some((me.uid(), me.gid())));
        assert_eq!(get("f").target, None);
        assert_eq!(get("l").target, Some("f".into()));
    }
}
