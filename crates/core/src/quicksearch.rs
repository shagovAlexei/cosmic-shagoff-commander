//! TC quick search / filter matching: name prefix, `*` = anywhere, case-insensitive.

use crate::mask::glob;
use crate::panel::PARENT;

/// `doc` → glob `doc*` (prefix); `*doc` → `*doc*` (anywhere): appending `*` covers both.
pub fn matches(pattern: &str, name: &str) -> bool {
    name != PARENT
        && glob(
            &format!("{}*", pattern.to_lowercase()),
            &name.to_lowercase(),
        )
}

/// A menu's letter key: the next label after `from` (round, so the same letter steps through
/// them) that starts with `c`, case-insensitive.
pub fn next_with<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    from: usize,
    c: char,
) -> Option<usize> {
    let labels: Vec<&str> = labels.into_iter().collect();
    let starts = |l: &str| {
        l.chars()
            .find(|ch| ch.is_alphanumeric())
            .is_some_and(|ch| ch.to_lowercase().eq(c.to_lowercase()))
    };
    (1..=labels.len())
        .map(|k| (from + k) % labels.len())
        .find(|&i| starts(labels[i]))
}

/// A TC / Windows menu name: `&x` marks the hot letter, `&&` is a plain `&`. The name as shown and
/// the hot letter (lowercase) with its byte offset in the shown name (to underline it).
pub fn hotkey(name: &str) -> (String, Option<(usize, char)>) {
    let (mut shown, mut key) = (String::new(), None);
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('&', Some('&')) => {
                shown.push('&');
                chars.next();
            }
            ('&', Some(k)) => key = key.or(k.to_lowercase().next().map(|l| (shown.len(), l))),
            ('&', None) => shown.push('&'),
            _ => shown.push(c),
        }
    }
    (shown, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_marks_and_escapes() {
        assert_eq!(hotkey("&Docs"), ("Docs".into(), Some((0, 'd'))));
        assert_eq!(hotkey("My &Work"), ("My Work".into(), Some((3, 'w'))));
        assert_eq!(hotkey("R&&D"), ("R&D".into(), None));
        assert_eq!(
            hotkey("Мои &Документы"),
            ("Мои Документы".into(), Some((7, 'д')))
        );
        assert_eq!(hotkey("a&b&c"), ("abc".into(), Some((1, 'b')))); // the first one counts
        assert_eq!(hotkey("end&"), ("end&".into(), None));
        assert_eq!(hotkey("plain"), ("plain".into(), None));
    }

    #[test]
    fn letter_steps_through_matches_round() {
        let l = ["Alpha", "", "beta", "‹ Back", "Bravo", "Аня"];
        assert_eq!(next_with(l, 0, 'b'), Some(2));
        assert_eq!(next_with(l, 2, 'B'), Some(3)); // the first letter, past "‹ "
        assert_eq!(next_with(l, 4, 'b'), Some(2)); // round
        assert_eq!(next_with(l, 0, 'а'), Some(5)); // Cyrillic
        assert_eq!(next_with(l, 0, 'a'), Some(0)); // the only one: stays
        assert_eq!(next_with(l, 0, 'z'), None);
        assert_eq!(next_with([], 0, 'a'), None);
    }

    #[test]
    fn prefix_match() {
        assert!(matches("doc", "documents"));
        assert!(!matches("doc", "my_doc"));
    }

    #[test]
    fn star_means_anywhere() {
        assert!(matches("*doc", "my_doc.txt"));
        assert!(matches("*.rs", "main.rs"));
        assert!(matches("a*z", "abcz"));
    }

    #[test]
    fn question_mark_is_one_char() {
        assert!(matches("f?o", "foo"));
        assert!(!matches("f?o", "fo"));
    }

    #[test]
    fn case_and_cyrillic() {
        assert!(matches("Док", "документы"));
        assert!(matches("док", "Документы"));
        assert!(matches("README", "readme.md"));
    }

    #[test]
    fn parent_row_never_matches() {
        assert!(!matches("", PARENT));
        assert!(!matches(".", PARENT));
        assert!(!matches("*", PARENT));
    }

    #[test]
    fn empty_pattern_matches_everything_else() {
        assert!(matches("", "anything"));
    }
}
