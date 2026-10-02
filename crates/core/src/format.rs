//! TC-style display formatting for the file table.

use crate::listing::Entry;
pub use jiff::tz::TimeZone;
use std::time::SystemTime;

/// `1204567` → `1 204 567`.
pub fn size(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// `1536` → `1.5 K`; binary units, one decimal below 100.
pub fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["K", "M", "G", "T", "P"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v < 100.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

/// `02.10.2026 13:49` in `tz`; empty if the time is out of range.
pub fn date(t: SystemTime, tz: &TimeZone) -> String {
    jiff::Timestamp::try_from(t)
        .map(|ts| {
            ts.to_zoned(tz.clone())
                .strftime("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Permission bits as `rwxr-xr-x` (file type bits ignored).
pub fn perms(mode: u32) -> String {
    (0..9)
        .map(|i| {
            let bit = 0o400 >> i;
            if mode & bit == 0 {
                '-'
            } else {
                ['r', 'w', 'x'][i % 3]
            }
        })
        .collect()
}

/// TC name column: dirs as `[name]`, files without the extension shown in its own column.
pub fn display_name(e: &Entry) -> String {
    if e.is_dir() {
        format!("[{}]", e.name)
    } else if e.ext.is_empty() {
        e.name.clone()
    } else {
        e.name[..e.name.len() - e.ext.len() - 1].to_string()
    }
}

/// Tab title: the last path component, `/` for the root.
pub fn dir_title(p: &std::path::Path) -> String {
    p.file_name()
        .map_or_else(|| "/".to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::Kind;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1536), "1.5 K");
        assert_eq!(human(12_900_000_000), "12.0 G");
        assert_eq!(human(450 * 1024 * 1024 * 1024), "450 G");
    }

    fn entry(name: &str, ext: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: ext.into(),
            size: 0,
            mtime: UNIX_EPOCH,
            kind,
            is_link: false,
            mode: 0,
        }
    }

    #[test]
    fn dir_title_last_component_or_root() {
        use std::path::Path;
        assert_eq!(dir_title(Path::new("/home/shag")), "shag");
        assert_eq!(dir_title(Path::new("/")), "/");
    }

    #[test]
    fn size_groups_thousands_with_spaces() {
        assert_eq!(size(0), "0");
        assert_eq!(size(999), "999");
        assert_eq!(size(1000), "1 000");
        assert_eq!(size(1_204_567), "1 204 567");
        assert_eq!(size(u64::MAX), "18 446 744 073 709 551 615");
    }

    #[test]
    fn date_in_given_zone() {
        let t = UNIX_EPOCH + Duration::from_secs(1_790_948_940); // 2026-10-02 13:49 UTC
        assert_eq!(date(t, &TimeZone::UTC), "02.10.2026 13:49");
        let msk = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(date(t, &msk), "02.10.2026 16:49");
    }

    #[test]
    fn perms_rwx() {
        assert_eq!(perms(0o755), "rwxr-xr-x");
        assert_eq!(perms(0o100640), "rw-r-----");
        assert_eq!(perms(0), "---------");
    }

    #[test]
    fn display_name_tc_style() {
        assert_eq!(display_name(&entry("src", "", Kind::Dir)), "[src]");
        assert_eq!(display_name(&entry("..", "", Kind::Dir)), "[..]");
        assert_eq!(
            display_name(&entry("Cargo.toml", "toml", Kind::File)),
            "Cargo"
        );
        assert_eq!(display_name(&entry("a.tar.gz", "gz", Kind::File)), "a.tar");
        assert_eq!(display_name(&entry(".bashrc", "", Kind::File)), ".bashrc");
        assert_eq!(display_name(&entry("noext", "", Kind::File)), "noext");
    }
}
