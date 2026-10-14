//! TC "Color by file type": rules giving file names a colour, the first matching one wins.

use crate::listing::Entry;
use crate::mask::Mask;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// What a rule applies to, besides its mask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum What {
    #[default]
    Any,
    Files,
    Dirs,
    Links,
    /// Files with an `x` bit.
    Exec,
}

impl What {
    pub const ALL: [What; 5] = [What::Any, What::Files, What::Dirs, What::Links, What::Exec];

    fn matches(self, e: &Entry) -> bool {
        match self {
            What::Any => true,
            What::Files => !e.is_dir(),
            What::Dirs => e.is_dir(),
            What::Links => e.is_link,
            // Not links: a dangling one has the link's own mode, 0o777.
            What::Exec => !e.is_dir() && !e.is_link && e.mode & 0o111 != 0,
        }
    }
}

/// `mask`: as Num+ (`*.rs;*.toml`), empty = every name. `color`: `#RRGGBB`, empty = the theme's
/// accent. `bg`: the row's background, empty = none. `size` (`>100M`, `<1k`, files only) and `age`
/// in days (`<7` newer than a week, `>365` older than a year), `attr` (letters, all must hold:
/// `h` hidden, `r` read-only, `s` setuid / setgid / sticky): empty = any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub mask: String,
    pub what: What,
    pub color: String,
    pub bg: String,
    pub size: String,
    pub age: String,
    pub attr: String,
}

/// What `attr` asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    pub hidden: bool,
    pub readonly: bool,
    pub special: bool,
}

/// `h r s` in any order and case (spaces and commas ignored); another letter → `None`.
pub fn attrs(s: &str) -> Option<Attrs> {
    let mut a = Attrs::default();
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != ',') {
        match c.to_ascii_lowercase() {
            'h' => a.hidden = true,
            'r' => a.readonly = true,
            's' => a.special = true,
            _ => return None,
        }
    }
    Some(a)
}

impl Attrs {
    fn fits(self, e: &Entry) -> bool {
        // A link's own mode is always 0o777: not read-only, never special.
        (!self.hidden || e.name.starts_with('.'))
            && (!self.readonly || (!e.is_link && e.mode & 0o200 == 0))
            && (!self.special || (!e.is_link && e.mode & 0o7000 != 0))
    }
}

/// Marked rows' text and background, the cursor's background (TC "Mark color", "Cursor color"):
/// `#RRGGBB`, empty = the theme's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkColors {
    pub color: String,
    pub bg: String,
    pub cursor: String,
}

/// `MarkColors` parsed; `None`: the theme's (empty or not a colour).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub fg: Option<[u8; 3]>,
    pub bg: Option<[u8; 3]>,
    pub cursor: Option<[u8; 3]>,
}

impl MarkColors {
    pub fn marks(&self) -> Marks {
        let rgb = |s: &str| match parse(s) {
            Some(Paint::Rgb(c)) => Some(c),
            _ => None,
        };
        Marks {
            fg: rgb(&self.color),
            bg: rgb(&self.bg),
            cursor: rgb(&self.cursor),
        }
    }
}

/// `>N` (more than) or `<N` (less than).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub more: bool,
    pub n: u64,
}

impl Bound {
    fn fits(self, v: u64) -> bool {
        if self.more { v > self.n } else { v < self.n }
    }
}

/// `>N` / `<N`, N with an optional `k` / `M` / `G` (1024-based, Latin or Cyrillic, any case) when
/// `units`; empty → `Some(None)`; anything else → `None`.
pub fn bound(s: &str, units: bool) -> Option<Option<Bound>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return Some(None);
    }
    let (more, rest) = match t.chars().next()? {
        '>' => (true, &t[1..]),
        '<' => (false, &t[1..]),
        _ => return None,
    };
    let last = rest.chars().last()?;
    let mul: u64 = match last.to_lowercase().next() {
        Some('k' | 'к') if units => 1 << 10,
        Some('m' | 'м') if units => 1 << 20,
        Some('g' | 'г') if units => 1 << 30,
        _ => 1,
    };
    let num = match mul {
        1 => rest,
        _ => &rest[..rest.len() - last.len_utf8()],
    };
    // Digits only: `parse` would also take a sign.
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Some(Bound {
        more,
        n: num.parse::<u64>().ok()?.checked_mul(mul)?,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Accent,
    Rgb([u8; 3]),
}

/// `#RRGGBB` (the `#` optional); empty → the accent; anything else → `None`.
pub fn parse(color: &str) -> Option<Paint> {
    let t = color.trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    if hex.is_empty() {
        return Some(Paint::Accent);
    }
    // Hex digits only: `from_str_radix` would also take a sign.
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Paint::Rgb([byte(0)?, byte(2)?, byte(4)?]))
}

/// The out-of-the-box list: links cyan, executables orange.
pub fn defaults() -> Vec<Rule> {
    let rule = |what, color: &str| Rule {
        mask: String::new(),
        what,
        color: color.into(),
        ..Default::default()
    };
    vec![rule(What::Links, "#33c7de"), rule(What::Exec, "#ff7800")]
}

/// Text on `bg` is better black (a light background) than white.
pub fn dark_text_on([r, g, b]: [u8; 3]) -> bool {
    299 * r as u32 + 587 * g as u32 + 114 * b as u32 > 128_000
}

/// What a fitting rule gives a row: the name's colour and, maybe, a background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// `None`: an empty colour on a background — the row's own text (black or white on it).
    pub fg: Option<Paint>,
    pub bg: Option<[u8; 3]>,
}

#[derive(Clone, Debug)]
struct Ready {
    mask: Option<Mask>,
    what: What,
    size: Option<Bound>,
    /// In days.
    age: Option<Bound>,
    attr: Attrs,
    look: Look,
}

impl Ready {
    fn fits(&self, e: &Entry, now: SystemTime) -> bool {
        // Dirs have no size; a date in the future is 0 days old.
        let days = || {
            now.duration_since(e.mtime)
                .unwrap_or(Duration::ZERO)
                .as_secs()
                / 86400
        };
        self.what.matches(e)
            && self.mask.as_ref().is_none_or(|m| m.matches(&e.name))
            && self.size.is_none_or(|b| !e.is_dir() && b.fits(e.size))
            && self.age.is_none_or(|b| b.fits(days()))
            && self.attr.fits(e)
    }
}

/// The rules ready to apply: masks and bounds parsed, bad rules dropped once, not per row and
/// frame.
#[derive(Clone, Debug, Default)]
pub struct Scheme(Vec<Ready>, pub Marks);

impl Rule {
    /// Parsed for use; `None` if its colour, background or a bound does not parse.
    fn ready(&self) -> Option<Ready> {
        let bg = match parse(&self.bg)? {
            Paint::Rgb(c) => Some(c),
            Paint::Accent => None,
        };
        Some(Ready {
            mask: (!self.mask.trim().is_empty()).then(|| Mask::parse(&self.mask)),
            what: self.what,
            size: bound(&self.size, true)?,
            age: bound(&self.age, false)?,
            attr: attrs(&self.attr)?,
            look: Look {
                fg: Some(parse(&self.color)?).filter(|p| bg.is_none() || *p != Paint::Accent),
                bg,
            },
        })
    }

    /// Every field parses (else the rule never applies).
    pub fn valid(&self) -> bool {
        self.ready().is_some()
    }
}

impl Scheme {
    pub fn new(rules: &[Rule]) -> Self {
        Self(
            rules.iter().filter_map(Rule::ready).collect(),
            Marks::default(),
        )
    }

    pub fn with_marks(self, m: &MarkColors) -> Self {
        Self(self.0, m.marks())
    }

    pub fn marks(&self) -> Marks {
        self.1
    }

    /// `e`'s look: the first rule whose kind, mask, size and age fit. `..` has none.
    pub fn pick(&self, e: &Entry, now: SystemTime) -> Option<Look> {
        if e.name == crate::panel::PARENT {
            return None;
        }
        self.0.iter().find(|r| r.fits(e, now)).map(|r| r.look)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(s: &Scheme, e: &Entry) -> Option<Paint> {
        s.pick(e, SystemTime::now()).and_then(|l| l.fg)
    }
    use crate::listing::Kind;
    use std::time::SystemTime;

    fn entry(name: &str, kind: Kind, link: bool, mode: u32) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: String::new(),
            size: 0,
            mtime: SystemTime::UNIX_EPOCH,
            kind,
            is_link: link,
            mode,
            owner: None,
            target: None,
        }
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse("#ff8000"), Some(Paint::Rgb([255, 128, 0])));
        assert_eq!(parse("00FF00"), Some(Paint::Rgb([0, 255, 0])));
        assert_eq!(parse(" "), Some(Paint::Accent));
        assert_eq!(parse("#fff"), None);
        assert_eq!(parse("#gg0000"), None);
        assert_eq!(parse("#ффф"), None);
        assert_eq!(parse("##ff0000"), None);
        assert_eq!(parse("#+1+2+3"), None);
    }

    #[test]
    fn the_first_fitting_rule_wins() {
        let rule = |mask: &str, what, color: &str| Rule {
            mask: mask.into(),
            what,
            color: color.into(),
            ..Default::default()
        };
        let rules = Scheme::new(&[
            rule("", What::Links, ""),
            rule("*.rs", What::Files, "#ff0000"),
            rule("", What::Dirs, "#0000ff"),
            rule("", What::Exec, "bad"), // never applies: the colour does not parse
            rule("", What::Exec, "#00ff00"),
        ]);
        let file = |n: &str, mode| entry(n, Kind::File, false, mode);
        assert_eq!(
            fg(&rules, &file("a.rs", 0o644)),
            Some(Paint::Rgb([255, 0, 0]))
        );
        assert_eq!(
            fg(&rules, &file("run", 0o755)),
            Some(Paint::Rgb([0, 255, 0]))
        );
        assert_eq!(fg(&rules, &file("a.txt", 0o644)), None);
        assert_eq!(fg(&rules, &entry("..", Kind::Dir, false, 0o755)), None);
        let broken = Scheme::new(&[rule("", What::Exec, "#00ff00")]);
        assert_eq!(fg(&broken, &entry("gone", Kind::File, true, 0o777)), None);
        assert_eq!(
            fg(&rules, &entry("a.rs", Kind::File, true, 0o644)),
            Some(Paint::Accent)
        );
        assert_eq!(
            fg(&rules, &entry("src", Kind::Dir, false, 0o755)),
            Some(Paint::Rgb([0, 0, 255]))
        );
        let stock = Scheme::new(&defaults());
        assert_eq!(fg(&stock, &file("a", 0o644)), None);
        assert_eq!(
            fg(&stock, &file("run", 0o755)),
            Some(Paint::Rgb([0xff, 0x78, 0]))
        );
        assert_eq!(
            fg(&stock, &entry("l", Kind::File, true, 0o777)),
            Some(Paint::Rgb([0x33, 0xc7, 0xde]))
        );
    }

    #[test]
    fn attributes() {
        assert_eq!(attrs(""), Some(Attrs::default()));
        assert_eq!(
            attrs("H, r"),
            Some(Attrs {
                hidden: true,
                readonly: true,
                special: false
            })
        );
        assert_eq!(attrs("hx"), None);
        let rule = |attr: &str| {
            Scheme::new(&[Rule {
                attr: attr.into(),
                color: "#ff0000".into(),
                ..Default::default()
            }])
        };
        let file = |n: &str, mode| entry(n, Kind::File, false, mode);
        let hit = |s: &Scheme, e: &Entry| fg(s, e).is_some();
        let (h, r, s, hr) = (rule("h"), rule("r"), rule("s"), rule("hr"));
        assert!(hit(&h, &file(".bashrc", 0o644)) && !hit(&h, &file("a", 0o644)));
        assert!(hit(&r, &file("a", 0o444)) && !hit(&r, &file("a", 0o644)));
        assert!(!hit(&r, &entry("l", Kind::File, true, 0o444))); // links: their own mode
        assert!(hit(&s, &file("su", 0o4755)) && hit(&s, &entry("tmp", Kind::Dir, false, 0o1777)));
        assert!(!hit(&s, &file("a", 0o755)));
        assert!(hit(&hr, &file(".a", 0o444)) && !hit(&hr, &file(".a", 0o644)));
        assert!(
            Scheme::new(&[Rule {
                attr: "q".into(),
                ..Default::default()
            }])
            .0
            .is_empty()
        );
    }

    #[test]
    fn mark_colours() {
        let m = MarkColors {
            color: "#ff0000".into(),
            bg: "".into(),
            cursor: "bad".into(),
        };
        assert_eq!(
            m.marks(),
            Marks {
                fg: Some([255, 0, 0]),
                bg: None,
                cursor: None
            }
        );
    }

    #[test]
    fn text_on_backgrounds() {
        assert!(dark_text_on([0xf9, 0xf0, 0x6b])); // light yellow
        assert!(dark_text_on([0x99, 0xc1, 0xf1])); // light blue
        assert!(!dark_text_on([0x1a, 0x5f, 0xb4])); // dark blue
        assert!(!dark_text_on([0, 0, 0]));
    }

    #[test]
    fn bounds_parse() {
        let b = |more, n| Some(Some(Bound { more, n }));
        assert_eq!(bound("", true), Some(None));
        assert_eq!(bound(">100", true), b(true, 100));
        assert_eq!(bound("< 2k", true), b(false, 2048));
        assert_eq!(bound(">1М", true), b(true, 1 << 20)); // Cyrillic
        assert_eq!(bound(">3g", true), b(true, 3 << 30));
        assert_eq!(bound("<7", false), b(false, 7));
        assert_eq!(bound("<7k", false), None); // days take no units
        assert_eq!(bound("7", true), None);
        assert_eq!(bound(">", true), None);
        assert_eq!(bound(">k", true), None);
        assert_eq!(bound(">+5", true), None);
        assert_eq!(bound(">99999999999999999999g", true), None);
    }

    #[test]
    fn size_age_and_background() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86400);
        let rule = |size: &str, age: &str, bg: &str| Rule {
            what: What::Any,
            color: "#ff0000".into(),
            bg: bg.into(),
            size: size.into(),
            age: age.into(),
            ..Default::default()
        };
        let at = |size, days_old: u64, kind| {
            let mut e = entry("x", kind, false, 0o644);
            e.size = size;
            e.mtime = now - Duration::from_secs(days_old * 86400);
            e
        };
        let big = Scheme::new(&[rule(">1k", "", "#00ff00")]);
        let look = big.pick(&at(2048, 0, Kind::File), now).unwrap();
        assert_eq!(look.bg, Some([0, 255, 0]));
        assert_eq!(big.pick(&at(1024, 0, Kind::File), now), None);
        assert_eq!(big.pick(&at(4096, 0, Kind::Dir), now), None); // dirs have no size
        let fresh = Scheme::new(&[rule("", "<7", "")]);
        assert_eq!(fresh.pick(&at(0, 6, Kind::Dir), now).unwrap().bg, None);
        let plain = Scheme::new(&[Rule {
            bg: "#ffffff".into(),
            ..Default::default()
        }]);
        let look = plain.pick(&at(0, 0, Kind::File), now).unwrap();
        assert_eq!((look.fg, look.bg), (None, Some([255, 255, 255]))); // accent → the row's text
        assert_eq!(fresh.pick(&at(0, 7, Kind::File), now), None);
        let mut future = at(0, 0, Kind::File);
        future.mtime = now + Duration::from_secs(86400);
        assert!(fresh.pick(&future, now).is_some());
        // A bad bound or background drops the rule.
        assert!(Scheme::new(&[rule("1k", "", "")]).0.is_empty());
        assert!(Scheme::new(&[rule("", "", "#zz0000")]).0.is_empty());
    }
}
