//! Alt+Enter: what a selection takes on disk, and changing permission bits.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
}

/// Bytes, files and dirs under `paths` (the dirs themselves counted), not following symlinks
/// (a link counts as a file of its own size). Unreadable entries are skipped. `None` = stopped.
pub fn usage(paths: &[PathBuf], stop: &AtomicBool) -> Option<Usage> {
    let mut u = Usage::default();
    let mut todo: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = todo.pop() {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(m) = fs::symlink_metadata(&p) else {
            continue;
        };
        if m.is_dir() {
            u.dirs += 1;
            if let Ok(rd) = fs::read_dir(&p) {
                todo.extend(rd.flatten().map(|e| e.path()));
            }
        } else {
            u.files += 1;
            u.bytes += m.len();
        }
    }
    Some(u)
}

/// Mode bits of `p` itself (`None` for a symlink: its own bits mean nothing on Linux).
pub fn mode(p: &Path) -> Option<u32> {
    let m = fs::symlink_metadata(p).ok()?;
    (!m.file_type().is_symlink()).then(|| m.permissions().mode() & 0o7777)
}

/// Turn on `set` and off `clear` (only the bits the user changed, so differing files keep the
/// rest) on `paths`, and under dirs too when `recursive`. Symlinks are left alone (chmod would
/// change their target). Returns what failed.
pub fn chmod(
    paths: &[PathBuf],
    set: u32,
    clear: u32,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    let mut failed = Vec::new();
    let mut todo: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = todo.pop() {
        let Ok(m) = fs::symlink_metadata(&p) else {
            continue;
        };
        if m.file_type().is_symlink() {
            continue;
        }
        let old = m.permissions().mode() & 0o7777;
        let new = (old & !clear) | (set & 0o7777);
        // Before descending: a dir being made readable must be listable.
        if new != old
            && let Err(e) = fs::set_permissions(&p, fs::Permissions::from_mode(new))
        {
            failed.push((p.clone(), e));
        }
        if recursive
            && m.is_dir()
            && let Ok(rd) = fs::read_dir(&p)
        {
            todo.extend(rd.flatten().map(|e| e.path()));
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn mode_of(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    fn set(p: &Path, m: u32) {
        fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
    }

    #[test]
    fn usage_counts_tree_without_following_links() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("a/b")).unwrap();
        fs::write(d.path().join("a/x"), "12345").unwrap();
        fs::write(d.path().join("a/b/y"), "123").unwrap();
        fs::write(d.path().join("z"), "1").unwrap();
        symlink(d.path().join("a"), d.path().join("a/loop")).unwrap();
        let u = usage(
            &[d.path().join("a"), d.path().join("z")],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!((u.dirs, u.files), (2, 4)); // a, b; x, y, loop, z
        assert!(u.bytes >= 9);
        assert_eq!(usage(&[d.path().into()], &AtomicBool::new(true)), None);
    }

    #[test]
    fn chmod_changes_only_toggled_bits() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();
        set(&a, 0o644);
        set(&b, 0o600);
        // +x for the owner, -r for others
        assert!(chmod(&[a.clone(), b.clone()], 0o100, 0o004, false).is_empty());
        assert_eq!((mode_of(&a), mode_of(&b)), (0o740, 0o700));
        assert_eq!(mode(&a), Some(0o740));
    }

    #[test]
    fn chmod_recursive_skips_links() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("top");
        fs::create_dir_all(top.join("sub")).unwrap();
        fs::write(top.join("sub/f"), "").unwrap();
        let outside = d.path().join("outside");
        fs::write(&outside, "").unwrap();
        set(&outside, 0o600);
        symlink(&outside, top.join("link")).unwrap();
        set(&top.join("sub/f"), 0o600);
        chmod(std::slice::from_ref(&top), 0o044, 0, false);
        assert_eq!(mode_of(&top.join("sub/f")), 0o600); // not recursive
        chmod(std::slice::from_ref(&top), 0o044, 0, true);
        assert_eq!(mode_of(&top.join("sub/f")), 0o644);
        assert_eq!(mode_of(&outside), 0o600); // the link's target untouched
        assert_eq!(mode(&top.join("link")), None);
    }
}
