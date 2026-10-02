//! Drive buttons: `/`, `~` and real block devices from `/proc/self/mounts`.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub label: String,
    pub path: PathBuf,
}

/// `/` and `home` first, then `/dev/*` mounts except loop/squashfs/`/boot*`, in mounts order.
/// A device mounted twice gives one drive: its `/media` or `/run/media` mount, else the first.
pub fn parse<'a>(mounts: &'a str, home: &Path) -> Vec<Drive> {
    let mut out = vec![
        Drive {
            label: "/".into(),
            path: "/".into(),
        },
        Drive {
            label: "~".into(),
            path: home.to_path_buf(),
        },
    ];
    let mut devices: Vec<&str> = Vec::new(); // devices[i] owns out[i + 2]
    let fields = |line: &'a str| {
        let mut f = line.split_whitespace();
        (f.next(), f.next(), f.next())
    };
    // The devices behind `/` and `~` already have a button, wherever else they are mounted.
    let builtin: Vec<&str> = mounts
        .lines()
        .filter_map(|l| match fields(l) {
            (Some(dev), Some(point), _)
                if unescape(point) == Path::new("/") || unescape(point) == home =>
            {
                Some(dev)
            }
            _ => None,
        })
        .collect();
    for line in mounts.lines() {
        let (Some(dev), Some(point), Some(fs)) = fields(line) else {
            continue;
        };
        if !dev.starts_with("/dev/")
            || dev.starts_with("/dev/loop")
            || fs == "squashfs"
            || builtin.contains(&dev)
        {
            continue;
        }
        let path = unescape(point);
        if path.starts_with("/boot") {
            continue;
        }
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let drive = Drive { label, path };
        match devices.iter().position(|d| *d == dev) {
            Some(i) => {
                if is_media(&drive.path) && !is_media(&out[i + 2].path) {
                    out[i + 2] = drive;
                }
            }
            None => {
                devices.push(dev);
                out.push(drive);
            }
        }
    }
    out
}

fn is_media(p: &Path) -> bool {
    p.starts_with("/media") || p.starts_with("/run/media")
}

/// The kernel escapes space, tab, newline and backslash in mounts as `\ooo` octal.
fn unescape(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(byte) = s
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    OsString::from_vec(out).into()
}

/// Index of the drive whose path is the longest (component-wise) prefix of `path`.
pub fn containing(drives: &[Drive], path: &Path) -> Option<usize> {
    drives
        .iter()
        .enumerate()
        .filter(|(_, d)| path.starts_with(&d.path))
        .max_by_key(|(_, d)| d.path.components().count())
        .map(|(i, _)| i)
}

/// (free for unprivileged users, total) bytes of the filesystem holding `path`.
pub fn space(path: &Path) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some((s.f_bavail * s.f_frsize, s.f_blocks * s.f_frsize))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from this machine's /proc/self/mounts.
    const MOUNTS: &str = "\
sysfs /sys sysfs rw,nosuid 0 0
/dev/nvme1n1p2 / ext4 rw,noatime 0 0
/dev/loop2 /snap/bare/5 squashfs ro 0 0
/dev/loop0 /snap/acestreamplayer/18 squashfs ro 0 0
/dev/nvme1n1p1 /boot/efi vfat rw 0 0
/dev/nvme1n1p4 /home ext4 rw 0 0
/dev/sda1 /mnt/save-flash-home vfat rw 0 0
/dev/sda1 /media/shag/SAVE_FLASH vfat rw 0 0
/dev/nvme0n1p3 /media/shag/sys fuseblk rw 0 0
tmpfs /run tmpfs rw 0 0
";

    fn labels(d: &[Drive]) -> Vec<&str> {
        d.iter().map(|d| d.label.as_str()).collect()
    }

    #[test]
    fn parse_keeps_root_home_and_real_devices() {
        let d = parse(MOUNTS, Path::new("/home/shag"));
        assert_eq!(labels(&d), ["/", "~", "home", "SAVE_FLASH", "sys"]);
        assert_eq!(d[1].path, Path::new("/home/shag"));
        assert_eq!(d[3].path, Path::new("/media/shag/SAVE_FLASH"));
    }

    #[test]
    fn parse_prefers_media_mount_for_a_device_mounted_twice() {
        let m = "/dev/sdb1 /media/u/X vfat rw 0 0\n/dev/sdb1 /mnt/x vfat rw 0 0\n";
        let d = parse(m, Path::new("/home/u"));
        assert_eq!(labels(&d), ["/", "~", "X"]);
        let m = "/dev/sdb1 /mnt/x vfat rw 0 0\n/dev/sdb1 /run/media/u/X vfat rw 0 0\n";
        let d = parse(m, Path::new("/home/u"));
        assert_eq!(d[2].path, Path::new("/run/media/u/X"));
    }

    #[test]
    fn parse_unescapes_spaces() {
        let d = parse(
            "/dev/sdb1 /media/u/My\\040Disk vfat rw 0 0\n",
            Path::new("/home/u"),
        );
        assert_eq!(d[2].label, "My Disk");
        assert_eq!(d[2].path, Path::new("/media/u/My Disk"));
    }

    #[test]
    fn parse_skips_root_and_home_repeats() {
        let m = "/dev/a / ext4 rw 0 0\n/dev/b /home/u ext4 rw 0 0\n";
        assert_eq!(labels(&parse(m, Path::new("/home/u"))), ["/", "~"]);
    }

    #[test]
    fn regression_root_or_home_device_mounted_again_gets_no_extra_button() {
        let m = "/dev/a / ext4 rw 0 0\n/dev/a /media/u/rootbind ext4 rw 0 0\n\
                 /dev/b /home/u ext4 rw 0 0\n/dev/b /run/media/u/homebind ext4 rw 0 0\n";
        assert_eq!(labels(&parse(m, Path::new("/home/u"))), ["/", "~"]);
    }

    #[test]
    fn containing_picks_longest_prefix_by_component() {
        let d = parse(MOUNTS, Path::new("/home/shag"));
        let label = |p: &str| containing(&d, Path::new(p)).map(|i| d[i].label.as_str());
        assert_eq!(label("/home/shag/x"), Some("~"));
        assert_eq!(label("/home/other"), Some("home"));
        assert_eq!(label("/media/shag/sys/a"), Some("sys"));
        assert_eq!(label("/etc"), Some("/"));
        assert_eq!(label("/homework"), Some("/"));
    }

    #[test]
    fn space_of_root_is_sane() {
        let (free, total) = space(Path::new("/")).expect("statvfs /");
        assert!(total > 0 && free <= total);
    }
}
