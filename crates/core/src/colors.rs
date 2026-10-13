//! TC "Color by file type": rules giving file names a colour, the first matching one wins.

use crate::listing::Entry;
use crate::mask::Mask;
use serde::{Deserialize, Serialize};

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
            What::Exec => !e.is_dir() && e.mode & 0o111 != 0,
        }
    }
}

/// `mask`: as Num+ (`*.rs;*.toml`), empty = every name. `color`: `#RRGGBB`, empty = the theme's
/// accent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub mask: String,
    pub what: What,
    pub color: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Accent,
    Rgb([u8; 3]),
}

/// `#RRGGBB` (the `#` optional); empty → the accent; anything else → `None`.
pub fn parse(color: &str) -> Option<Paint> {
    let hex = color.trim().trim_start_matches('#');
    if hex.is_empty() {
        return Some(Paint::Accent);
    }
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Paint::Rgb([byte(0)?, byte(2)?, byte(4)?]))
}

/// The out-of-the-box list: links in the accent colour (as before the rules existed).
pub fn defaults() -> Vec<Rule> {
    vec![Rule {
        mask: String::new(),
        what: What::Links,
        color: String::new(),
    }]
}

/// The colour of `e`'s name: the first rule whose kind and mask fit and whose colour parses.
// ponytail: masks are parsed per row per frame; cache them if long rule lists get slow.
pub fn pick(rules: &[Rule], e: &Entry) -> Option<Paint> {
    rules
        .iter()
        .filter(|r| r.what.matches(e))
        .filter(|r| r.mask.trim().is_empty() || Mask::parse(&r.mask).matches(&e.name))
        .find_map(|r| parse(&r.color))
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }

    #[test]
    fn the_first_fitting_rule_wins() {
        let rule = |mask: &str, what, color: &str| Rule {
            mask: mask.into(),
            what,
            color: color.into(),
        };
        let rules = [
            rule("", What::Links, ""),
            rule("*.rs", What::Files, "#ff0000"),
            rule("", What::Dirs, "#0000ff"),
            rule("", What::Exec, "bad"), // never applies: the colour does not parse
            rule("", What::Exec, "#00ff00"),
        ];
        let file = |n: &str, mode| entry(n, Kind::File, false, mode);
        assert_eq!(
            pick(&rules, &file("a.rs", 0o644)),
            Some(Paint::Rgb([255, 0, 0]))
        );
        assert_eq!(
            pick(&rules, &file("run", 0o755)),
            Some(Paint::Rgb([0, 255, 0]))
        );
        assert_eq!(pick(&rules, &file("a.txt", 0o644)), None);
        assert_eq!(
            pick(&rules, &entry("a.rs", Kind::File, true, 0o644)),
            Some(Paint::Accent)
        );
        assert_eq!(
            pick(&rules, &entry("src", Kind::Dir, false, 0o755)),
            Some(Paint::Rgb([0, 0, 255]))
        );
        assert_eq!(pick(&defaults(), &file("a", 0o644)), None);
    }
}
