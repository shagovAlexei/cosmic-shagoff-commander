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

/// `1536` → `1.5 K` with `units` = [bytes, K, M, G, T, P] and `sep` as decimal separator.
/// One decimal below 100; rounds first, so 99.96 G is `100 G` and 1023.97 K is `1.0 M`.
pub fn human(n: u64, units: &[&str; 6], sep: char) -> String {
    if n < 1024 {
        return format!("{n} {}", units[0]);
    }
    let mut v = n as f64;
    let mut unit = 0;
    loop {
        v /= 1024.0;
        unit += 1;
        let tenths = (v * 10.0).round() / 10.0;
        let shown = if tenths < 100.0 { tenths } else { v.round() };
        if shown < 1024.0 || unit == units.len() - 1 {
            let num = if shown < 100.0 {
                format!("{shown:.1}").replace('.', &sep.to_string())
            } else {
                format!("{shown:.0}")
            };
            return format!("{num} {}", units[unit]);
        }
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

/// Status bar: `name → target   1 204 567   02.10.2026 13:49   rwxr-xr-x (755)   user:group`.
/// Empty for `..`; no size for dirs; no owner for archive entries.
pub fn details(e: &Entry, tz: &TimeZone, owners: &crate::owners::Owners) -> String {
    if e.name == crate::panel::PARENT {
        return String::new();
    }
    // stat failed (dir readable but not searchable): nothing but the name is known.
    if e.mode == 0 && e.owner.is_none() {
        return e.name.clone();
    }
    let mut parts = vec![match &e.target {
        Some(t) => format!("{} → {}", e.name, t.display()),
        None => e.name.clone(),
    }];
    if !e.is_dir() {
        parts.push(size(e.size));
    }
    parts.push(date(e.mtime, tz));
    parts.push(format!("{} ({:o})", perms(e.mode), e.mode & 0o7777));
    if let Some((uid, gid)) = e.owner {
        parts.push(owners.name(uid, gid));
    }
    parts.join("   ")
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

    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];

    #[test]
    fn human_sizes() {
        let h = |n| human(n, &UNITS, '.');
        assert_eq!(h(0), "0 B");
        assert_eq!(h(1023), "1023 B");
        assert_eq!(h(1536), "1.5 K");
        assert_eq!(h(12_900_000_000), "12.0 G");
        assert_eq!(h(450 * 1024 * 1024 * 1024), "450 G");
    }

    #[test]
    fn human_uses_given_units_and_separator() {
        let ru = ["Б", "КБ", "МБ", "ГБ", "ТБ", "ПБ"];
        assert_eq!(human(13_207_024_435, &ru, ','), "12,3 ГБ");
    }

    #[test]
    fn regression_human_rounds_before_choosing_unit() {
        let h = |n| human(n, &UNITS, '.');
        assert_eq!(h(107_331_578_920), "100 G"); // 99.96 G, not "100.0 G"
        assert_eq!(h(1_048_545), "1.0 M"); // 1023.97 K, not "1024 K"
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
            owner: None,
            target: None,
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

    fn full(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: String::new(),
            size: 1204567,
            mtime: UNIX_EPOCH + Duration::from_secs(86_400),
            kind,
            is_link: false,
            mode: 0o100755,
            owner: Some((0, 0)),
            target: None,
        }
    }

    #[test]
    fn details_line() {
        let tz = TimeZone::UTC;
        let o = crate::owners::Owners::default();
        assert_eq!(
            details(&full("a.sh", Kind::File), &tz, &o),
            "a.sh   1 204 567   02.01.1970 00:00   rwxr-xr-x (755)   0:0"
        );
        let mut d = full("docs", Kind::Dir);
        d.owner = None; // inside an archive
        assert_eq!(
            details(&d, &tz, &o),
            "docs   02.01.1970 00:00   rwxr-xr-x (755)"
        );
        let mut l = full("lib", Kind::File);
        l.is_link = true;
        l.target = Some("/usr/lib/x".into());
        assert!(details(&l, &tz, &o).starts_with("lib → /usr/lib/x   1 204 567"));
        assert_eq!(details(&full("..", Kind::Dir), &tz, &o), "");
        let mut unreadable = full("secret", Kind::File); // stat failed (dir r-- without x)
        (unreadable.mode, unreadable.owner) = (0, None);
        assert_eq!(details(&unreadable, &tz, &o), "secret");
    }
}
