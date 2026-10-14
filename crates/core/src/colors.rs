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
            // Not links: a dangling one has the link's own mode, 0o777.
            What::Exec => !e.is_dir() && !e.is_link && e.mode & 0o111 != 0,
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
    };
    vec![rule(What::Links, "#33c7de"), rule(What::Exec, "#ff7800")]
}

/// The rules ready to apply: masks parsed and bad colours dropped once, not per row and frame.
#[derive(Clone, Debug, Default)]
pub struct Scheme(Vec<(Option<Mask>, What, Paint)>);

impl Scheme {
    pub fn new(rules: &[Rule]) -> Self {
        Self(
            rules
                .iter()
                .filter_map(|r| {
                    let mask = (!r.mask.trim().is_empty()).then(|| Mask::parse(&r.mask));
                    Some((mask, r.what, parse(&r.color)?))
                })
                .collect(),
        )
    }

    /// The colour of `e`'s name: the first rule whose kind and mask fit. `..` has none.
    pub fn pick(&self, e: &Entry) -> Option<Paint> {
        if e.name == crate::panel::PARENT {
            return None;
        }
        self.0
            .iter()
            .find(|(m, w, _)| w.matches(e) && m.as_ref().is_none_or(|m| m.matches(&e.name)))
            .map(|(_, _, p)| *p)
    }
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
        assert_eq!(parse("##ff0000"), None);
        assert_eq!(parse("#+1+2+3"), None);
    }

    #[test]
    fn the_first_fitting_rule_wins() {
        let rule = |mask: &str, what, color: &str| Rule {
            mask: mask.into(),
            what,
            color: color.into(),
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
            rules.pick(&file("a.rs", 0o644)),
            Some(Paint::Rgb([255, 0, 0]))
        );
        assert_eq!(
            rules.pick(&file("run", 0o755)),
            Some(Paint::Rgb([0, 255, 0]))
        );
        assert_eq!(rules.pick(&file("a.txt", 0o644)), None);
        assert_eq!(rules.pick(&entry("..", Kind::Dir, false, 0o755)), None);
        let broken = Scheme::new(&[rule("", What::Exec, "#00ff00")]);
        assert_eq!(broken.pick(&entry("gone", Kind::File, true, 0o777)), None);
        assert_eq!(
            rules.pick(&entry("a.rs", Kind::File, true, 0o644)),
            Some(Paint::Accent)
        );
        assert_eq!(
            rules.pick(&entry("src", Kind::Dir, false, 0o755)),
            Some(Paint::Rgb([0, 0, 255]))
        );
        let stock = Scheme::new(&defaults());
        assert_eq!(stock.pick(&file("a", 0o644)), None);
        assert_eq!(
            stock.pick(&file("run", 0o755)),
            Some(Paint::Rgb([0xff, 0x78, 0]))
        );
        assert_eq!(
            stock.pick(&entry("l", Kind::File, true, 0o777)),
            Some(Paint::Rgb([0x33, 0xc7, 0xde]))
        );
    }
}
