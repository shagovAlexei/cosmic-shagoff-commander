//! A key combination as the user's own hotkeys keep it (`Ctrl+Shift+K`): parsed, and written back
//! the same way. The key is a name: a letter or digit by its place on the keyboard (`K` is also
//! `Л` on the Russian layout), `F1`…`F12`, `Enter`, `Up`, `PageDown`…, or any other key's code
//! name (`Backquote`, `NumpadAdd`).

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bind {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Bind {
    /// `Ctrl+Alt+Shift+Key`, any order and case of the modifiers; `None` without a key or with an
    /// unknown modifier.
    pub fn parse(s: &str) -> Option<Bind> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last()?;
        let mut b = Bind {
            ctrl: false,
            alt: false,
            shift: false,
            key: normalize(key)?,
        };
        for m in mods {
            match m.to_lowercase().as_str() {
                "ctrl" | "control" => b.ctrl = true,
                "alt" => b.alt = true,
                "shift" => b.shift = true,
                _ => return None,
            }
        }
        Some(b)
    }
}

/// A one-letter name upper-cased (`k` → `K`); anything else as given. Empty or with spaces: none.
fn normalize(key: &str) -> Option<String> {
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let mut c = key.chars();
    Some(match (c.next(), c.next()) {
        (Some(ch), None) => ch.to_uppercase().collect(),
        _ => key.to_string(),
    })
}

impl fmt::Display for Bind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_any_order_and_writes_back_one_way() {
        let b = Bind::parse("shift + ctrl+k").unwrap();
        assert_eq!(
            (b.ctrl, b.alt, b.shift, b.key.as_str()),
            (true, false, true, "K")
        );
        assert_eq!(b.to_string(), "Ctrl+Shift+K");
        assert_eq!(Bind::parse("F5").unwrap().to_string(), "F5");
        assert_eq!(
            Bind::parse("Alt+Control+PageDown").unwrap().to_string(),
            "Ctrl+Alt+PageDown"
        );
        assert_eq!(Bind::parse("Ctrl+Backquote").unwrap().key, "Backquote");
    }

    #[test]
    fn rejects_no_key_and_unknown_modifiers() {
        assert_eq!(Bind::parse(""), None);
        assert_eq!(Bind::parse("Ctrl+"), None);
        assert_eq!(Bind::parse("Super+K"), None);
        assert_eq!(Bind::parse("Ctrl+Page Down"), None);
    }
}
