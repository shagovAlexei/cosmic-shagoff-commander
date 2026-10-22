//! The trash as a dir (TC `cm_OpenRecycled`): the freedesktop trash's `files/` is listed like any
//! other dir; each top-level item has `info/<name>.trashinfo` saying where it came from. Deleting
//! there is for good (and drops the info), «Restore» moves an item back.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The home trash's dir (`$XDG_DATA_HOME/Trash`), by the environment only.
fn home_root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| Path::new(d).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    Some(data.join("Trash"))
}

/// The home trash's `files/`, made if missing (gio makes it on the first delete; the trash is
/// opened before that too), private as gio makes it.
pub fn home() -> io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let root = home_root().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let mut b = fs::DirBuilder::new();
    b.recursive(true).mode(0o700);
    b.create(root.join("info"))?;
    let files = root.join("files");
    b.create(&files)?;
    Ok(files)
}

/// `dir` is a trash's `files/` (by its path only: called while drawing): the home trash's, or a
/// drive's of this user (`top/.Trash-1000/files`, `top/.Trash/1000/files`) — not just any
/// `Trash/files` (an unpacked archive could carry one).
pub fn is_top(dir: &Path) -> bool {
    let Some(root) = dir
        .parent()
        .filter(|_| dir.file_name() == Some(OsStr::new("files")))
    else {
        return false;
    };
    let uid = rustix::process::geteuid().as_raw().to_string();
    let name = root.file_name().map(OsStr::as_bytes);
    home_root().is_some_and(|h| h == root)
        || name == Some(format!(".Trash-{uid}").as_bytes())
        || (name == Some(uid.as_bytes())
            && root.parent().and_then(Path::file_name) == Some(OsStr::new(".Trash")))
}

/// `dir` is in a trash (its `files/` or below): deleting there is for good.
pub fn inside(dir: &Path) -> bool {
    dir.ancestors().any(is_top)
}

/// An item right in a trash's `files/` → its info file.
fn info_of(item: &Path) -> Option<PathBuf> {
    let files = item.parent().filter(|d| is_top(d))?;
    let mut name = item.file_name()?.to_owned();
    name.push(".trashinfo");
    Some(files.parent()?.join("info").join(name))
}

/// Where the item right in a trash's `files/` was deleted from.
pub fn origin(item: &Path) -> Option<PathBuf> {
    let text = fs::read(info_of(item)?).ok()?;
    let line = text
        .split(|&b| b == b'\n')
        .find_map(|l| l.strip_prefix(b"Path="))?;
    let path = PathBuf::from(OsString::from_vec(unescape(line.trim_ascii_end())));
    let root = item.parent()?.parent()?;
    if home_root().is_some_and(|h| h == root) {
        return path.is_absolute().then_some(path); // the home trash keeps whole paths
    }
    // A drive's trash: relative to the drive's top (`top/.Trash-1000`, `top/.Trash/1000`); an
    // item there is never put back off the drive.
    let top = if root.file_name()?.as_bytes().starts_with(b".Trash-") {
        root.parent()?
    } else {
        root.parent()?.parent()?
    };
    let path = top.join(path);
    (path.starts_with(top)
        && !path
            .components()
            .any(|c| c == std::path::Component::ParentDir))
    .then_some(path)
}

/// `%2F` → `/` (the info file keeps the path URL-escaped).
fn unescape(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%'
            && let (Some(h), Some(l)) = (
                s.get(i + 1).and_then(|&b| hex(b)),
                s.get(i + 2).and_then(|&b| hex(b)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// An item right in a trash's `files/` was deleted for good: its info goes too.
pub fn forget(item: &Path) {
    if let Some(info) = info_of(item) {
        let _ = fs::remove_file(info);
    }
}

/// Moves an item right in a trash's `files/` back where it was deleted from (its dir made again
/// if gone); an existing file there is never replaced. Returns where it went.
pub fn restore(item: &Path) -> io::Result<PathBuf> {
    let to = origin(item).ok_or_else(|| io::Error::other(crate::why::NOT_IN_TRASH))?;
    if fs::symlink_metadata(&to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    // ponytail: a rename only — every trash is on its item's own drive; EXDEV is reported.
    crate::ops::rename_noreplace(item, &to)?;
    forget(item);
    Ok(to)
}

/// Every item in every trash (the home one and the drives'), for «Empty trash».
pub fn everything() -> io::Result<Vec<PathBuf>> {
    let roots = trash::os_limited::trash_folders().map_err(io::Error::other)?;
    let mut out: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| fs::read_dir(r.join("files")).ok())
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
pub(crate) fn test_root(dir: &Path) -> PathBuf {
    dir.join(format!(".Trash-{}", rustix::process::geteuid().as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trash(root: &Path, name: &str, path: &str) -> PathBuf {
        fs::create_dir_all(root.join("files")).unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        let item = root.join("files").join(name);
        fs::write(&item, "x").unwrap();
        let info = format!("[Trash Info]\nPath={path}\nDeletionDate=2026-10-22T10:00:00\n");
        fs::write(root.join("info").join(format!("{name}.trashinfo")), info).unwrap();
        item
    }

    #[test]
    fn a_trash_is_told_by_its_path() {
        let uid = rustix::process::geteuid().as_raw();
        assert!(is_top(&home_root().unwrap().join("files")));
        assert!(is_top(&test_root(Path::new("/media/u/usb")).join("files")));
        assert!(is_top(
            &Path::new("/media/u/usb/.Trash")
                .join(uid.to_string())
                .join("files")
        ));
        // Someone else's, or just a dir with that name (an unpacked archive's).
        let other = format!("/media/u/usb/.Trash-{}/files", uid + 1);
        assert!(!is_top(Path::new(&other)));
        assert!(!is_top(Path::new("/home/u/Projects/Trash/files")));
        assert!(!is_top(Path::new("/home/u/files")));
        let sub = test_root(Path::new("/m")).join("files/sub");
        assert!(!is_top(&sub) && inside(&sub));
        assert!(!inside(&test_root(Path::new("/m"))));
    }

    #[test]
    fn an_item_goes_back_where_it_was_deleted_from() {
        let t = tempfile::tempdir().unwrap();
        let root = test_root(t.path());
        let home = t.path().join("home");
        let gone = home.join("my docs/a b.txt");
        // The path is escaped: a space is `%20`.
        let esc = gone.to_str().unwrap().replace(' ', "%20");
        let item = trash(&root, "a b.2.txt", &esc);
        assert_eq!(origin(&item), Some(gone.clone()));
        // Its dir is gone too: made again.
        assert_eq!(restore(&item).unwrap(), gone);
        assert_eq!(fs::read_to_string(&gone).unwrap(), "x");
        assert!(!item.exists());
        assert!(!root.join("info/a b.2.txt.trashinfo").exists());
        // Taken: never replaced, the item stays in the trash.
        let again = trash(&root, "a b.txt", &esc);
        fs::write(&gone, "new").unwrap();
        assert_eq!(
            restore(&again).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&gone).unwrap(), "new");
        assert!(again.exists() && root.join("info/a b.txt.trashinfo").exists());
        // Deleted for good: the info goes with it.
        fs::remove_file(&again).unwrap();
        forget(&again);
        assert!(!root.join("info/a b.txt.trashinfo").exists());
        // Not right in a trash: nothing to restore.
        assert!(restore(&home.join("my docs")).is_err());
    }

    #[test]
    fn a_drive_trash_keeps_paths_on_the_drive() {
        let t = tempfile::tempdir().unwrap();
        let item = trash(&test_root(t.path()), "x", "dir/x");
        assert_eq!(origin(&item), Some(t.path().join("dir/x")));
        let uid = rustix::process::geteuid().as_raw().to_string();
        let item = trash(&t.path().join(".Trash").join(uid), "y", "y");
        assert_eq!(origin(&item), Some(t.path().join("y")));
        // Never off the drive.
        let item = trash(&test_root(t.path()), "z", "/etc/z");
        assert_eq!(origin(&item), None);
        let item = trash(&test_root(t.path()), "w", "../w");
        assert_eq!(origin(&item), None);
    }
}
