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

#[cfg(test)]
mod tests {
    use super::*;

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
