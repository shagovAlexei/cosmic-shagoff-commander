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

#[cfg(test)]
mod tests {
    use super::*;

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
