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

/// Whether size `n` is shown in KB / MB / GB: as `chosen`, else as the skin has it — `modern`
/// always, the classic one from 1 000 000 000 up (no longer fits its column).
pub fn human_size(n: u64, chosen: Option<bool>, modern: bool) -> bool {
    chosen.unwrap_or(modern || n >= 1_000_000_000)
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

/// A date as the «change date» field takes it: `2026-10-23 14:05:07`.
pub fn date_edit(t: SystemTime, tz: &TimeZone) -> String {
    jiff::Timestamp::try_from(t)
        .map(|ts| {
            ts.to_zoned(tz.clone())
                .strftime("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// `date_edit`'s text back (seconds optional; `DD.MM.YYYY` as the panels show it also), in `tz`.
pub fn parse_date(text: &str, tz: &TimeZone) -> Option<SystemTime> {
    let text = text.trim();
    let parse = |fmt: &str| {
        jiff::fmt::strtime::parse(fmt, text)
            .ok()?
            .to_datetime()
            .ok()
    };
    let dt = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%d.%m.%Y %H:%M:%S",
        "%d.%m.%Y %H:%M",
    ]
    .iter()
    .find_map(|f| parse(f))?;
    let ts = dt.to_zoned(tz.clone()).ok()?.timestamp();
    Some(SystemTime::from(ts))
}

/// Permission bits as `rwxr-xr-x` (file type bits ignored).
pub fn perms(mode: u32) -> String {
    (0..9)
        .map(|i| {
            let on = mode & (0o400 >> i) != 0;
            // The x of owner / group / others carries setuid / setgid / sticky, as `ls`.
            let special = i % 3 == 2 && mode & (0o4000 >> (i / 3)) != 0;
            match (on, special) {
                (true, false) => ['r', 'w', 'x'][i % 3],
                (false, false) => '-',
                (x, true) => {
                    let c = if i == 8 { 't' } else { 's' };
                    if x { c } else { c.to_ascii_uppercase() }
                }
            }
        })
        .collect()
}

/// Attr column, as `ls -l`: the type (`d` dir, `l` symlink, `-` other), then `perms` (of the
/// link's target for a link).
pub fn attrs(e: &Entry) -> String {
    let kind = if e.is_link {
        'l'
    } else if e.is_dir() {
        'd'
    } else {
        '-'
    };
    format!("{kind}{}", perms(e.mode))
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
/// `size`: a file's size as shown (bytes or KB / MB).
pub fn details(
    e: &Entry,
    tz: &TimeZone,
    owners: &crate::owners::Owners,
    size: impl Fn(u64) -> String,
) -> String {
    if e.name == crate::panel::PARENT {
        return String::new();
    }
    // stat failed (dir readable but not searchable): nothing but the name is known.
    if e.unknown() {
        return e.name.clone();
    }
    // In the trash (a target that is no link): where it came from is the line — the rest is in
    // the columns, and with it the path was cut to nothing.
    if let Some(t) = e.target.as_ref().filter(|_| !e.is_link) {
        return format!("{} ← {}", e.name, t.display());
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

/// MIME type guessed from a file extension (`rs` → `text/x-rust`).
pub fn mime_type(ext: &str) -> Option<String> {
    (!ext.is_empty())
        .then(|| mime_guess::from_ext(ext).first())
        .flatten()
        .map(|m| m.essence_str().to_string())
}

/// Theme icon for a row and the generic one to fall back to (freedesktop names): `folder`, `go-up`
/// for `..`, else the MIME type guessed from the extension (`text/x-rust` → `text-x-rust`).
pub fn icon_name(e: &Entry) -> (String, &'static str) {
    if e.name == crate::panel::PARENT {
        return ("go-up".into(), "go-up");
    }
    if e.is_dir() {
        return ("folder".into(), "folder");
    }
    match mime_guess::from_ext(&e.ext).first() {
        Some(m) if !e.ext.is_empty() => {
            let generic = match m.type_().as_str() {
                "image" => "image-x-generic",
                "audio" => "audio-x-generic",
                "video" => "video-x-generic",
                "font" => "font-x-generic",
                _ => "text-x-generic",
            };
            (m.essence_str().replace('/', "-"), generic)
        }
        _ if e.mode & 0o111 != 0 => ("application-x-executable".into(), "text-x-generic"),
        _ => ("text-x-generic".into(), "text-x-generic"),
    }
}

/// TC «Copy file details» / «Save all shown columns to file»: a line per row, its columns as
/// shown (name, extension, size, date, attributes) between tabs; `head`: their titles first.
/// A row is (entry, its Size column as the panel shows it); a row with no details (`unknown`)
/// has an empty date and attributes, as there.
pub fn details_table(rows: &[(&Entry, String)], tz: &TimeZone, head: Option<[&str; 5]>) -> String {
    let line = |e: &Entry, size: &str| {
        let (date, attrs) = match e.unknown() {
            true => (String::new(), String::new()),
            false => (date(e.mtime, tz), attrs(e)),
        };
        [
            display_name(e),
            e.ext.clone(),
            size.to_string(),
            date,
            attrs,
        ]
        .join("\t")
    };
    (head.map(|h| h.join("\t")).into_iter())
        .chain(rows.iter().map(|(e, s)| line(e, s)))
        .map(|l| l + "\n")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::Kind;
    use std::time::{Duration, UNIX_EPOCH};

    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];

    #[test]
    fn human_size_as_chosen_else_by_skin() {
        assert!(!human_size(999_999_999, None, false));
        assert!(human_size(1_000_000_000, None, false));
        assert!(human_size(1, None, true));
        assert!(!human_size(5 << 30, Some(false), true));
        assert!(human_size(1, Some(true), false));
    }

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
    fn icon_names_by_kind_and_type() {
        let name = |e: &Entry| icon_name(e);
        assert_eq!(name(&entry("..", "", Kind::Dir)), ("go-up".into(), "go-up"));
        assert_eq!(
            name(&entry("src", "", Kind::Dir)),
            ("folder".into(), "folder")
        );
        assert_eq!(name(&entry("a.rs", "rs", Kind::File)).0, "text-x-rust");
        assert_eq!(
            name(&entry("a.png", "png", Kind::File)),
            ("image-png".into(), "image-x-generic")
        );
        assert_eq!(
            name(&entry("a.zip", "zip", Kind::File)).0,
            "application-zip"
        );
        assert_eq!(
            name(&entry("a.qqq", "qqq", Kind::File)),
            ("text-x-generic".into(), "text-x-generic")
        );
        let mut exe = entry("run", "", Kind::File);
        exe.mode = 0o755;
        assert_eq!(name(&exe).0, "application-x-executable");
        assert_eq!(name(&entry("README", "", Kind::File)).0, "text-x-generic");
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
        // As `ls`: setuid / setgid `s` over x, `S` without it; sticky `t` / `T`.
        assert_eq!(perms(0o4755), "rwsr-xr-x");
        assert_eq!(perms(0o2644), "rw-r-Sr--");
        assert_eq!(perms(0o41777), "rwxrwxrwt");
        assert_eq!(perms(0o1770), "rwxrwx--T");
        let mut e = Entry {
            name: "a".into(),
            os_name: "a".into(),
            ext: String::new(),
            size: 0,
            mtime: std::time::SystemTime::UNIX_EPOCH,
            kind: crate::listing::Kind::File,
            is_link: false,
            mode: 0o644,
            owner: None,
            target: None,
        };
        assert_eq!(attrs(&e), "-rw-r--r--");
        e.is_link = true;
        assert_eq!(attrs(&e), "lrw-r--r--");
        (e.is_link, e.kind) = (false, crate::listing::Kind::Dir);
        assert_eq!(attrs(&e), "drw-r--r--");
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
            details(&full("a.sh", Kind::File), &tz, &o, self::size),
            "a.sh   1 204 567   02.01.1970 00:00   rwxr-xr-x (755)   0:0"
        );
        let mut d = full("docs", Kind::Dir);
        d.owner = None; // inside an archive
        assert_eq!(
            details(&d, &tz, &o, self::size),
            "docs   02.01.1970 00:00   rwxr-xr-x (755)"
        );
        let mut l = full("lib", Kind::File);
        l.is_link = true;
        l.target = Some("/usr/lib/x".into());
        assert!(details(&l, &tz, &o, self::size).starts_with("lib → /usr/lib/x   1 204 567"));
        assert_eq!(details(&full("..", Kind::Dir), &tz, &o, self::size), "");
        let mut unreadable = full("secret", Kind::File); // stat failed (dir r-- without x)
        // As `listing::scan` makes it.
        (unreadable.mode, unreadable.owner) = (0, None);
        (unreadable.size, unreadable.mtime) = (0, UNIX_EPOCH);
        assert_eq!(details(&unreadable, &tz, &o, self::size), "secret");
    }

    #[test]
    fn regression_the_trash_origin_is_not_cut_away() {
        // In the trash: where it was deleted from, nothing else (with size, date, mode and owner
        // after it, the half-width status line cut the path to `/h…`).
        let o = crate::owners::Owners::default();
        let mut t = full("a.txt", Kind::File);
        t.target = Some("/home/u/docs/a.txt".into());
        let line = details(&t, &TimeZone::UTC, &o, self::size);
        assert_eq!(line, "a.txt ← /home/u/docs/a.txt");
    }

    #[test]
    fn dates_are_edited_and_read_back() {
        let tz = TimeZone::fixed(jiff::tz::offset(3));
        let t = parse_date("2026-10-23 14:05:07", &tz).unwrap();
        assert_eq!(date_edit(t, &tz), "2026-10-23 14:05:07");
        assert_eq!(
            parse_date("23.10.2026 14:05", &tz),
            parse_date("2026-10-23 14:05:00", &tz)
        );
        assert_eq!(parse_date("2026-13-40 99:99", &tz), None);
        assert_eq!(parse_date("tomorrow", &tz), None);
    }

    #[test]
    fn details_are_the_shown_columns_by_tabs() {
        use crate::listing::Kind;
        let e = |name: &str, ext: &str, kind| Entry {
            name: name.into(),
            os_name: name.into(),
            ext: ext.into(),
            size: 1234,
            mtime: SystemTime::UNIX_EPOCH,
            kind,
            is_link: false,
            mode: 0o644,
            owner: None,
            target: None,
        };
        let (f, d) = (e("a.txt", "txt", Kind::File), e("sub", "", Kind::Dir));
        let utc = TimeZone::UTC;
        let text = details_table(&[(&f, "1 234".into()), (&d, "<DIR>".into())], &utc, None);
        assert_eq!(
            text,
            "a\ttxt\t1 234\t01.01.1970 00:00\t-rw-r--r--\n[sub]\t\t<DIR>\t01.01.1970 00:00\tdrw-r--r--\n"
        );
        let head = details_table(
            &[(&f, "1 234".into())],
            &utc,
            Some(["Name", "Ext", "Size", "Date", "Attr"]),
        );
        assert!(head.starts_with("Name\tExt\tSize\tDate\tAttr\na\t"));
    }
}
