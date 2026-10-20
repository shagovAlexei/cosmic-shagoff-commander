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
/// rest) on `paths`, and under dirs too when `recursive`; setuid / setgid are put only on `paths`
/// and the dirs under them, never on the files inside (setgid for a tree's dirs must not make
/// every program in it set-group-id) — sticky and taking bits off reach everything. Symlinks are left alone (chmod would change their
/// target). Returns what failed, unreadable dirs included, and bits the kernel dropped silently
/// (setgid when not in the file's group).
pub fn chmod(
    paths: &[PathBuf],
    set: u32,
    clear: u32,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    enum Step {
        /// The path, and whether it was selected (not found inside a selected dir).
        Enter(PathBuf, bool),
        /// A dir's final mode, after its contents: taking away r / x first would lock them out.
        Finish(PathBuf, u32),
    }
    let mut failed = Vec::new();
    let apply = |p: &Path, old: u32, new: u32, failed: &mut Vec<(PathBuf, io::Error)>| {
        if new == old {
            return;
        }
        match fs::set_permissions(p, fs::Permissions::from_mode(new)) {
            Err(e) => failed.push((p.to_path_buf(), e)),
            // Linux drops setgid without an error when the user is not in the file's group.
            Ok(()) if mode(p).is_some_and(|m| m != new) => failed.push((
                p.to_path_buf(),
                io::Error::other(format!("mode is {:o}, not {new:o}", mode(p).unwrap_or(0))),
            )),
            Ok(()) => {}
        }
    };
    let mut todo: Vec<Step> = (paths.iter().rev())
        .map(|p| Step::Enter(p.clone(), true))
        .collect();
    while let Some(step) = todo.pop() {
        let (p, top) = match step {
            Step::Enter(p, top) => (p, top),
            Step::Finish(p, new) => {
                let old = mode(&p).unwrap_or(new);
                apply(&p, old, new, &mut failed);
                continue;
            }
        };
        let m = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) => {
                failed.push((p, e));
                continue;
            }
        };
        if m.file_type().is_symlink() {
            continue;
        }
        // Files found inside: no setuid / setgid put on (every program in the tree would run as
        // its owner / group); sticky and taking any of them off are harmless.
        let set_mask = if top || m.is_dir() { 0o7777 } else { 0o1777 };
        let (set, clear) = (set & set_mask, clear & 0o7777);
        let old = m.permissions().mode() & 0o7777;
        let new = (old & !clear) | set;
        if !(recursive && m.is_dir()) {
            apply(&p, old, new, &mut failed);
            continue;
        }
        // Added bits first (a dir being opened up must be listable), the rest after its contents.
        apply(&p, old, old | set, &mut failed);
        let kids = match fs::read_dir(&p) {
            Ok(rd) => rd.flatten().map(|e| e.path()).collect::<Vec<_>>(),
            Err(e) => {
                failed.push((p.clone(), e));
                Vec::new()
            }
        };
        todo.push(Step::Finish(p, new));
        todo.extend(kids.into_iter().map(|k| Step::Enter(k, false)));
    }
    failed
}

/// Give `paths` (and with `recursive` everything under the dirs) the owner `uid` and / or the
/// group `gid`. Links get them themselves, never their targets (`lchown`). Entries that already
/// have them are left alone. The kernel drops setuid / setgid on a chown: put back (root may, and
/// a non-root user changes only the group of his own files). Returns what failed (without root:
/// not the owner, not one of the user's groups; unreadable dirs).
pub fn chown(
    paths: &[PathBuf],
    uid: Option<u32>,
    gid: Option<u32>,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    use std::os::unix::fs::MetadataExt;
    let mut failed = Vec::new();
    let mut todo: Vec<PathBuf> = paths.iter().rev().cloned().collect();
    while let Some(p) = todo.pop() {
        let m = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) => {
                failed.push((p, e));
                continue;
            }
        };
        if uid.is_some_and(|u| u != m.uid()) || gid.is_some_and(|g| g != m.gid()) {
            match std::os::unix::fs::lchown(&p, uid, gid) {
                Err(e) => failed.push((p.clone(), e)),
                Ok(()) if !m.file_type().is_symlink() && m.mode() & 0o6000 != 0 => {
                    let back = fs::Permissions::from_mode(m.mode() & 0o7777);
                    if let Err(e) = fs::set_permissions(&p, back) {
                        failed.push((p.clone(), e));
                    }
                }
                Ok(()) => {}
            }
        }
        if recursive && m.is_dir() {
            match fs::read_dir(&p) {
                Ok(rd) => todo.extend(rd.flatten().map(|e| e.path())),
                Err(e) => failed.push((p, e)),
            }
        }
    }
    failed
}

/// What Alt+Enter's OK does: the owner / group first (a chown drops setuid / setgid), then the
/// toggled bits, which put back what was asked.
pub fn apply(
    paths: &[PathBuf],
    uid: Option<u32>,
    gid: Option<u32>,
    (set, clear): (u32, u32),
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    let mut failed = Vec::new();
    if uid.is_some() || gid.is_some() {
        failed = chown(paths, uid, gid, recursive);
    }
    if set | clear != 0 {
        failed.extend(chmod(paths, set, clear, recursive));
    }
    failed
}

/// The first argument of the program run as root by pkexec to do `apply` (163).
pub const HELPER: &str = "--props-apply";

/// Arguments after the program for `HELPER`: `uid gid set clear recursive -- paths`, `-` for
/// no owner / group, the bits in octal.
pub fn helper_args(
    paths: &[PathBuf],
    uid: Option<u32>,
    gid: Option<u32>,
    (set, clear): (u32, u32),
    recursive: bool,
) -> Vec<std::ffi::OsString> {
    let id = |i: Option<u32>| i.map_or("-".to_string(), |i| i.to_string());
    let mut v: Vec<std::ffi::OsString> = [
        HELPER.to_string(),
        id(uid),
        id(gid),
        format!("{set:o}"),
        format!("{clear:o}"),
        u8::from(recursive).to_string(),
        "--".into(),
    ]
    .map(Into::into)
    .into();
    v.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
    v
}

/// `HELPER`'s side: parse `helper_args` (`args[0]` = `HELPER`) and `apply`. Returns the
/// failures as lines, or why the arguments are wrong.
pub fn run_helper(args: &[std::ffi::OsString]) -> Result<Vec<String>, String> {
    let bad = || "bad arguments".to_string();
    let [_, uid, gid, set, clear, rec, dashes, paths @ ..] = args else {
        return Err(bad());
    };
    let s = |a: &std::ffi::OsString| a.to_str().map(str::to_string).ok_or_else(bad);
    let id = |a| -> Result<Option<u32>, String> {
        match s(a)?.as_str() {
            "-" => Ok(None),
            n => n.parse().map(Some).map_err(|_| bad()),
        }
    };
    let bits = |a| u32::from_str_radix(&s(a)?, 8).map_err(|_| bad());
    if dashes != "--" || paths.is_empty() {
        return Err(bad());
    }
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let failed = apply(
        &paths,
        id(uid)?,
        id(gid)?,
        (bits(set)? & 0o7777, bits(clear)? & 0o7777),
        s(rec)? == "1",
    );
    Ok((failed.into_iter())
        .map(|(p, e)| format!("{}: {e}", p.display()))
        .collect())
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
    fn recursive_setuid_setgid_go_to_dirs_not_to_the_files_inside() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("top");
        fs::create_dir_all(top.join("sub")).unwrap();
        fs::write(top.join("prog"), "").unwrap();
        set(&top.join("prog"), 0o755);
        // setgid + group w through the tree: the dirs get both, the file only w.
        assert!(chmod(std::slice::from_ref(&top), 0o2020, 0, true).is_empty());
        assert_eq!(mode_of(&top) & 0o2020, 0o2020);
        assert_eq!(mode_of(&top.join("sub")) & 0o2020, 0o2020);
        assert_eq!(mode_of(&top.join("prog")), 0o775);
        // regression: sticky goes to the files inside too, and taking bits off reaches them.
        assert!(chmod(std::slice::from_ref(&top), 0o1000, 0, true).is_empty());
        assert_eq!(mode_of(&top.join("prog")), 0o1775);
        set(&top.join("prog"), 0o6775); // set by hand before
        assert!(chmod(std::slice::from_ref(&top), 0, 0o7000, true).is_empty());
        assert_eq!(mode_of(&top.join("prog")), 0o775);
        assert_eq!(mode_of(&top) & 0o7000, 0);
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
        // The special bits as the rest: setuid on a file, then off.
        assert!(chmod(std::slice::from_ref(&a), 0o4000, 0, false).is_empty());
        assert_eq!(mode(&a), Some(0o4740));
        assert!(chmod(std::slice::from_ref(&a), 0, 0o4000, false).is_empty());
        assert_eq!(mode(&a), Some(0o740));
    }

    #[test]
    fn chmod_recursive_closing_a_dir_still_reaches_inside() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("top");
        fs::create_dir_all(top.join("sub")).unwrap();
        fs::write(top.join("sub/f"), "").unwrap();
        set(&top.join("sub/f"), 0o755);
        // take r and x away from the owner on everything
        let failed = chmod(std::slice::from_ref(&top), 0, 0o500, true);
        assert!(failed.is_empty(), "{failed:?}");
        set(&top, 0o700); // let the test look inside again
        set(&top.join("sub"), 0o700);
        assert_eq!(mode_of(&top.join("sub/f")), 0o255);
    }

    #[test]
    fn chmod_recursive_both_ways() {
        let t = tempfile::tempdir().unwrap();
        let (d, s) = (t.path().join("d"), t.path().join("d/s"));
        fs::create_dir_all(&s).unwrap();
        let (f, g) = (d.join("f"), s.join("g"));
        fs::write(&f, "").unwrap();
        fs::write(&g, "").unwrap();
        for (p, m) in [(&d, 0o755), (&s, 0o755), (&f, 0o644), (&g, 0o644)] {
            set(p, m);
        }
        let all = |m_dir: u32, m_file: u32| {
            assert_eq!(
                [mode_of(&d), mode_of(&s), mode_of(&f), mode_of(&g)],
                [m_dir, m_dir, m_file, m_file]
            );
        };
        assert!(chmod(std::slice::from_ref(&d), 0, 0o044, true).is_empty());
        all(0o711, 0o600);
        assert!(chmod(std::slice::from_ref(&d), 0o044, 0, true).is_empty());
        all(0o755, 0o644);
        // Owner r-x off locks us out of looking; turning it back on must still reach inside.
        assert!(chmod(std::slice::from_ref(&d), 0, 0o500, true).is_empty());
        assert!(chmod(std::slice::from_ref(&d), 0o500, 0, true).is_empty());
        all(0o755, 0o744); // files get x too: only touched bits, the same on every entry
    }

    #[test]
    fn chgrp_to_own_group_and_refused_otherwise() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("top");
        fs::create_dir_all(top.join("sub")).unwrap();
        fs::write(top.join("sub/f"), "").unwrap();
        symlink("/etc/passwd", top.join("ln")).unwrap();
        let mine = crate::owners::my_groups();
        let now = fs::metadata(&top).unwrap().gid();
        // Another of the user's groups, so the change really happens (one group: as it is).
        let g = mine.iter().copied().find(|g| *g != now).unwrap_or(now);
        set(&top.join("sub/f"), 0o6755); // kept through the change
        assert!(chown(std::slice::from_ref(&top), None, Some(g), true).is_empty());
        assert_eq!(mode_of(&top.join("sub/f")), 0o6755);
        for p in [top.clone(), top.join("sub/f"), top.join("ln")] {
            assert_eq!(fs::symlink_metadata(&p).unwrap().gid(), g, "{p:?}");
        }
        assert_ne!(fs::metadata("/etc/passwd").unwrap().gid(), g); // the link's target untouched
        // A group the user is not in: refused (unless root, who may do anything).
        if rustix::process::geteuid().as_raw() != 0
            && let Some(other) = (0..65534).find(|g| !mine.contains(g))
        {
            let failed = chown(std::slice::from_ref(&top), None, Some(other), false);
            assert_eq!(failed.len(), 1);
        }
    }

    #[test]
    fn chmod_reports_an_unreadable_dir() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("top");
        fs::create_dir_all(top.join("locked")).unwrap();
        set(&top.join("locked"), 0o300); // no r: cannot be listed
        let failed = chmod(std::slice::from_ref(&top), 0o004, 0, true);
        set(&top.join("locked"), 0o700);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, top.join("locked"));
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

    #[test]
    fn helper_round_trip_applies_owner_group_and_bits() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("-f");
        fs::write(&f, "x").unwrap();
        set(&f, 0o600);
        let m = fs::metadata(&f).unwrap();
        // Its own owner and group: no root needed, the bits still applied.
        let args = helper_args(
            std::slice::from_ref(&f),
            Some(m.uid()),
            Some(m.gid()),
            (0o044, 0o200),
            true,
        );
        assert_eq!(run_helper(&args), Ok(vec![]));
        assert_eq!(mode_of(&f), 0o444);
        let args = helper_args(&[d.path().join("none")], None, Some(m.gid()), (0, 0), false);
        assert_eq!(run_helper(&args).unwrap().len(), 1);
        assert!(run_helper(&args[..5]).is_err());
    }
}
