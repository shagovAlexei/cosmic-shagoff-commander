//! Ctrl+Z: what the last operation did, and how to take it back. The engine (`ops::Report.undo`)
//! and F7 (`mkdir`) write it; the app keeps the last one and carries it out.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::ops::Method;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Moved / renamed (by this method): (where it is now, where it was), the last done first.
    /// Undone by moving each back in this order (Ctrl+M chains and swaps names).
    Moved(Method, Vec<(PathBuf, PathBuf)>),
    /// Copies made: undone by putting them in the trash.
    Copied(Vec<PathBuf>),
    /// Put in the trash from these paths: undone by restoring them.
    Trashed(Vec<PathBuf>),
    /// F7: the dirs it made, the deepest first: undone by removing them while empty.
    Made(Vec<PathBuf>),
    /// Something was replaced or deleted for good: a `why` text saying so.
    Lost(&'static str),
}

/// F7: `create_dir_all(dir)`, returning the dirs it made, the deepest first (none told for a
/// name with `.` / `..`: its ancestors are not the dirs made).
pub fn mkdir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let plain = dir
        .components()
        .all(|c| !matches!(c, Component::CurDir | Component::ParentDir));
    let made: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|d| fs::symlink_metadata(d).is_err())
        .map(Path::to_path_buf)
        .filter(|_| plain)
        .collect();
    fs::create_dir_all(dir)?;
    Ok(made)
}

/// Removes F7's dirs, the deepest first; one that is not empty stops it (nothing above goes,
/// and those stay for the next Ctrl+Z).
pub fn unmake(dirs: &[PathBuf]) -> io::Result<()> {
    for d in dirs {
        match fs::remove_dir(d) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => {
                return Err(io::Error::other(crate::why::NOT_EMPTY));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mkdir_tells_what_it_made_and_unmake_removes_only_that() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("a")).unwrap();
        let made = mkdir(&t.path().join("a/b/c")).unwrap();
        assert_eq!(made, vec![t.path().join("a/b/c"), t.path().join("a/b")]);
        // Already there: nothing made.
        assert_eq!(mkdir(&t.path().join("a/b")).unwrap(), Vec::<PathBuf>::new());
        assert_eq!(
            mkdir(&t.path().join("x/../y")).unwrap(),
            Vec::<PathBuf>::new()
        );
        // Something put inside meanwhile: kept, with the dirs above it.
        fs::write(t.path().join("a/b/c/x"), "x").unwrap();
        assert_eq!(
            unmake(&made).unwrap_err().to_string(),
            crate::why::NOT_EMPTY
        );
        assert!(t.path().join("a/b/c/x").exists());
        fs::remove_file(t.path().join("a/b/c/x")).unwrap();
        unmake(&made).unwrap();
        assert!(!t.path().join("a/b").exists() && t.path().join("a").exists());
    }
}
