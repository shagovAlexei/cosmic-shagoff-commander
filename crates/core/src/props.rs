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

/// Set the modification and / or access time (`None`: left as it is) of `paths`, and of everything
/// under dirs when `recursive` (a dir after its contents: writing into it would move its time
/// again); a symlink gets its own times. Returns what failed. Creation and status-change times
/// cannot be set on Linux.
pub fn set_times(
    paths: &[PathBuf],
    modified: Option<std::time::SystemTime>,
    accessed: Option<std::time::SystemTime>,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    fn spec(t: Option<std::time::SystemTime>) -> rustix::fs::Timespec {
        match t {
            None => rustix::fs::Timespec {
                tv_sec: 0,
                tv_nsec: rustix::fs::UTIME_OMIT,
            },
            Some(t) => {
                let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                rustix::fs::Timespec {
                    tv_sec: d.as_secs() as _,
                    tv_nsec: d.subsec_nanos() as _,
                }
            }
        }
    }
    type Times = (Option<std::time::SystemTime>, Option<std::time::SystemTime>);
    fn one(p: &Path, t: Times, recursive: bool, failed: &mut Vec<(PathBuf, io::Error)>) {
        let Ok(meta) = fs::symlink_metadata(p) else {
            return;
        };
        if recursive && meta.is_dir() {
            match fs::read_dir(p) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        one(&e.path(), t, true, failed);
                    }
                }
                Err(e) => failed.push((p.to_path_buf(), e)),
            }
        }
        // By path, not through an open file (a file we own but cannot read is fine).
        let ts = rustix::fs::Timestamps {
            last_access: spec(t.1),
            last_modification: spec(t.0),
        };
        let done = rustix::fs::utimensat(
            rustix::fs::CWD,
            p,
            &ts,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        );
        if let Err(e) = done {
            failed.push((p.to_path_buf(), e.into()));
        }
    }
    let mut failed = Vec::new();
    for p in paths {
        one(p, (modified, accessed), recursive, &mut failed);
    }
    failed
}

/// Mode bits of `p` itself (`None` for a symlink: its own bits mean nothing on Linux).
pub fn mode(p: &Path) -> Option<u32> {
    let m = fs::symlink_metadata(p).ok()?;
    (!m.file_type().is_symlink()).then(|| m.permissions().mode() & 0o7777)
}

/// An entry reached without following links (pkexec runs `chmod` / `chown` as root over other
/// users' trees: a file swapped for a symlink between the look and the change must not send root
/// elsewhere). Named relative to its dir's descriptor, held open since that dir was read.
struct At {
    /// `None`: a path the user chose (its dirs are theirs); else the dir it was listed in.
    dir: Option<std::sync::Arc<rustix::fd::OwnedFd>>,
    name: std::ffi::OsString,
    /// For the failures shown.
    path: PathBuf,
}

impl At {
    fn top(p: &Path) -> Self {
        At {
            dir: None,
            name: p.as_os_str().to_owned(),
            path: p.to_path_buf(),
        }
    }

    /// `O_PATH | O_NOFOLLOW`: the entry itself (a link is the link), whatever its mode.
    fn open(&self) -> io::Result<rustix::fd::OwnedFd> {
        use rustix::fs::{CWD, Mode, OFlags, openat};
        let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        Ok(match &self.dir {
            Some(d) => openat(&**d, &*self.name, flags, Mode::empty())?,
            None => openat(CWD, &*self.name, flags, Mode::empty())?,
        })
    }
}

/// `chmod` of what `fd` (`O_PATH`) is, through `/proc/self/fd`: no path walked again.
fn set_mode(fd: &rustix::fd::OwnedFd, mode: u32) -> io::Result<()> {
    use rustix::fd::AsRawFd;
    let link = format!("/proc/self/fd/{}", fd.as_raw_fd());
    Ok(rustix::fs::chmod(
        link,
        rustix::fs::Mode::from_raw_mode(mode),
    )?)
}

/// The entries of the dir `fd` (`O_PATH`), each relative to it.
fn kids(fd: &std::sync::Arc<rustix::fd::OwnedFd>, path: &Path) -> io::Result<Vec<At>> {
    use rustix::fs::{Dir, Mode, OFlags, openat};
    use std::os::unix::ffi::OsStrExt;
    let d = openat(
        &**fd,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut out = Vec::new();
    for e in Dir::read_from(&d)? {
        let e = e?;
        let name = std::ffi::OsStr::from_bytes(e.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        out.push(At {
            dir: Some(fd.clone()),
            name: name.to_owned(),
            path: path.join(name),
        });
    }
    Ok(out)
}

/// Turn on `set` and off `clear` (only the bits the user changed, so differing files keep the
/// rest) on `paths`, and under dirs too when `recursive`; setuid / setgid are put only on `paths`
/// and the dirs under them, never on the files inside (setgid for a tree's dirs must not make
/// every program in it set-group-id) — sticky and taking bits off reach everything. Symlinks are
/// left alone (chmod would change their target); nothing is reached through one (`At`). Returns
/// what failed, unreadable dirs included, and bits the kernel dropped silently (setgid when not
/// in the file's group).
pub fn chmod(
    paths: &[PathBuf],
    set: u32,
    clear: u32,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    use rustix::fd::OwnedFd;
    use std::sync::Arc;
    enum Step {
        /// The entry, and whether it was selected (not found inside a selected dir).
        Enter(At, bool),
        /// A dir's final mode, after its contents: taking away r / x first would lock them out.
        Finish(Arc<OwnedFd>, PathBuf, u32),
    }
    let mut failed = Vec::new();
    let apply =
        |fd: &OwnedFd, p: &Path, old: u32, new: u32, failed: &mut Vec<(PathBuf, io::Error)>| {
            if new == old {
                return;
            }
            let now = || rustix::fs::fstat(fd).map(|st| st.st_mode & 0o7777);
            match set_mode(fd, new) {
                Err(e) => failed.push((p.to_path_buf(), e)),
                // Linux drops setgid without an error when the user is not in the file's group.
                Ok(()) if now().is_ok_and(|m| m != new) => failed.push((
                    p.to_path_buf(),
                    io::Error::other(format!("mode is {:o}, not {new:o}", now().unwrap_or(0))),
                )),
                Ok(()) => {}
            }
        };
    let mut todo: Vec<Step> = (paths.iter().rev())
        .map(|p| Step::Enter(At::top(p), true))
        .collect();
    while let Some(step) = todo.pop() {
        let (at, top) = match step {
            Step::Enter(at, top) => (at, top),
            Step::Finish(fd, p, new) => {
                let old = rustix::fs::fstat(&*fd).map_or(new, |st| st.st_mode & 0o7777);
                apply(&fd, &p, old, new, &mut failed);
                continue;
            }
        };
        let fd = match at.open() {
            Ok(fd) => fd,
            Err(e) => {
                failed.push((at.path, e));
                continue;
            }
        };
        let st = match rustix::fs::fstat(&fd) {
            Ok(st) => st,
            Err(e) => {
                failed.push((at.path, e.into()));
                continue;
            }
        };
        let kind = rustix::fs::FileType::from_raw_mode(st.st_mode);
        if kind == rustix::fs::FileType::Symlink {
            continue;
        }
        let is_dir = kind == rustix::fs::FileType::Directory;
        // Files found inside: no setuid / setgid put on (every program in the tree would run as
        // its owner / group); sticky and taking any of them off are harmless.
        let set_mask = if top || is_dir { 0o7777 } else { 0o1777 };
        let (set, clear) = (set & set_mask, clear & 0o7777);
        let old = st.st_mode & 0o7777;
        let new = (old & !clear) | set;
        if !(recursive && is_dir) {
            apply(&fd, &at.path, old, new, &mut failed);
            continue;
        }
        // Added bits first (a dir being opened up must be listable), the rest after its contents.
        apply(&fd, &at.path, old, old | set, &mut failed);
        let fd = Arc::new(fd);
        let inside = match kids(&fd, &at.path) {
            Ok(k) => k,
            Err(e) => {
                failed.push((at.path.clone(), e));
                Vec::new()
            }
        };
        todo.push(Step::Finish(fd, at.path, new));
        todo.extend(inside.into_iter().map(|k| Step::Enter(k, false)));
    }
    failed
}

/// Give `paths` (and with `recursive` everything under the dirs) the owner `uid` and / or the
/// group `gid`. Links get them themselves, never their targets, and nothing is reached through
/// one (`At`). Entries that already have them are left alone. The kernel drops setuid / setgid on
/// a chown: put back (root may, and a non-root user changes only the group of his own files).
/// Returns what failed (without root: not the owner, not one of the user's groups; unreadable
/// dirs).
pub fn chown(
    paths: &[PathBuf],
    uid: Option<u32>,
    gid: Option<u32>,
    recursive: bool,
) -> Vec<(PathBuf, io::Error)> {
    use rustix::fs::{AtFlags, FileType, Gid, Uid, chownat, fstat};
    let mut failed = Vec::new();
    let mut todo: Vec<At> = paths.iter().rev().map(|p| At::top(p)).collect();
    while let Some(at) = todo.pop() {
        let st = at.open().and_then(|fd| Ok((fstat(&fd)?, fd)));
        let (st, fd) = match st {
            Ok(s) => s,
            Err(e) => {
                failed.push((at.path, e));
                continue;
            }
        };
        let kind = FileType::from_raw_mode(st.st_mode);
        if uid.is_some_and(|u| u != st.st_uid) || gid.is_some_and(|g| g != st.st_gid) {
            // `AT_EMPTY_PATH` on the `O_PATH` descriptor: this very entry (a link: the link).
            let r = chownat(
                &fd,
                "",
                uid.map(Uid::from_raw),
                gid.map(Gid::from_raw),
                AtFlags::EMPTY_PATH,
            );
            match r {
                Err(e) => failed.push((at.path.clone(), e.into())),
                Ok(()) if kind != FileType::Symlink && st.st_mode & 0o6000 != 0 => {
                    if let Err(e) = set_mode(&fd, st.st_mode & 0o7777) {
                        failed.push((at.path.clone(), e));
                    }
                }
                Ok(()) => {}
            }
        }
        if recursive && kind == FileType::Directory {
            match kids(&std::sync::Arc::new(fd), &at.path) {
                Ok(k) => todo.extend(k),
                Err(e) => failed.push((at.path, e)),
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

    #[test]
    fn times_are_set_on_files_and_under_dirs_and_on_a_link_itself() {
        use std::time::{Duration, UNIX_EPOCH};
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("f"), "x").unwrap();
        fs::write(d.path().join("g"), "x").unwrap();
        symlink(d.path().join("g"), d.path().join("l")).unwrap();
        let to = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let m = |p: PathBuf| fs::symlink_metadata(p).unwrap().modified().unwrap();
        // Not recursive: the dir itself only.
        assert!(set_times(std::slice::from_ref(&sub), Some(to), None, false).is_empty());
        assert_eq!(m(sub.clone()), to);
        assert_ne!(m(sub.join("f")), to);
        // Recursive; a link gets its own time, its target keeps its.
        let l = d.path().join("l");
        assert!(set_times(&[sub.clone(), l.clone()], Some(to), None, true).is_empty());
        assert_eq!((m(sub.join("f")), m(sub.clone()), m(l)), (to, to, to));
        assert_ne!(m(d.path().join("g")), to);
        // The access time alone leaves the modification time.
        let at = UNIX_EPOCH + Duration::from_secs(1_100_000_000);
        let g = d.path().join("g");
        assert!(set_times(std::slice::from_ref(&g), None, Some(at), false).is_empty());
        let meta = fs::metadata(&g).unwrap();
        assert_eq!(meta.accessed().unwrap(), at);
        assert_ne!(meta.modified().unwrap(), at);
    }

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
    fn regression_root_chmod_chown_never_go_through_a_link() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let (tree, outside) = (d.path().join("tree"), d.path().join("outside"));
        fs::create_dir_all(&tree).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), "").unwrap();
        set(&outside.join("secret"), 0o600);
        set(&outside, 0o700);
        symlink(&outside, tree.join("dir")).unwrap();
        symlink(outside.join("secret"), tree.join("file")).unwrap();
        fs::write(tree.join("own"), "").unwrap();
        set(&tree.join("own"), 0o600);
        let failed = chmod(std::slice::from_ref(&tree), 0o066, 0, true);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(mode_of(&tree.join("own")), 0o666);
        assert_eq!(
            (mode_of(&outside), mode_of(&outside.join("secret"))),
            (0o700, 0o600)
        );
        // chown to what it is already: walks the tree, never into the link.
        let gid = fs::metadata(&tree).unwrap().gid();
        assert!(chown(std::slice::from_ref(&tree), None, Some(gid), true).is_empty());
        // A link given itself is the link: chmod leaves it, the target stays.
        assert!(chmod(&[tree.join("file")], 0o077, 0, false).is_empty());
        assert_eq!(mode_of(&outside.join("secret")), 0o600);
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
