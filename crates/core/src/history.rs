//! Per-tab directory history for Alt+← / Alt+→ / Alt+↓.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LIMIT: usize = 50;

/// Saved with the tabs (`PaneState.history`), so Alt+↓ survives a restart as in TC.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    items: Vec<PathBuf>,
    pos: usize,
}

impl History {
    /// Now at `path`: no-op if it is the current entry; else drop "forward", push, trim to LIMIT.
    pub fn visit(&mut self, path: &Path) {
        if self.items.get(self.pos).is_some_and(|p| p == path) {
            return;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.pos + 1);
        }
        self.items.push(path.to_path_buf());
        if self.items.len() > LIMIT {
            self.items.remove(0);
        }
        self.pos = self.items.len() - 1;
    }

    /// Step back; the dir to open, or None at the oldest entry.
    pub fn back(&mut self) -> Option<PathBuf> {
        // Read back from the state file: `pos` may be past the end (a hand edit, a cut write).
        self.pos = self.pos.min(self.items.len().saturating_sub(1));
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        Some(self.items[self.pos].clone())
    }

    /// Step forward; the dir to open, or None at the newest entry.
    pub fn forward(&mut self) -> Option<PathBuf> {
        if self.pos + 1 >= self.items.len() {
            return None;
        }
        self.pos += 1;
        Some(self.items[self.pos].clone())
    }

    /// A step (Alt+← / Alt+→) whose dir could not be read: back on `shown`, the dir still on
    /// screen (the entry nearest the step), so the next visit cuts "forward" after it, not after
    /// the dir that failed.
    pub fn seat(&mut self, shown: &Path) {
        if self.items.get(self.pos).is_some_and(|p| p == shown) {
            return;
        }
        let near = (0..self.items.len())
            .filter(|&i| self.items[i] == shown)
            .min_by_key(|&i| i.abs_diff(self.pos));
        if let Some(i) = near {
            self.pos = i;
        }
    }

    /// Forget everything but the dir the tab is in.
    pub fn clear(&mut self) {
        let current = self.items.get(self.pos).cloned();
        self.items = current.into_iter().collect();
        self.pos = 0;
    }

    /// Unique dirs: the current one first, then the rest by most recent visit.
    pub fn recent(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in self
            .items
            .get(self.pos)
            .into_iter()
            .chain(self.items.iter().rev())
        {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(paths: &[&str]) -> History {
        let mut h = History::default();
        for p in paths {
            h.visit(Path::new(p));
        }
        h
    }

    #[test]
    fn clear_keeps_only_the_current_dir() {
        let mut x = h(&["/a", "/b", "/c"]);
        x.back();
        x.clear();
        assert_eq!(x.recent(), [PathBuf::from("/b")]);
        assert_eq!((x.back(), x.forward()), (None, None));
        x.visit(Path::new("/d"));
        assert_eq!(x.back(), Some(PathBuf::from("/b")));
        let mut empty = History::default();
        empty.clear();
        assert!(empty.recent().is_empty());
    }

    #[test]
    fn back_and_forward_walk_the_visits() {
        let mut h = h(&["/a", "/b", "/c"]);
        assert_eq!(h.back(), Some("/b".into()));
        assert_eq!(h.back(), Some("/a".into()));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some("/b".into()));
        assert_eq!(h.forward(), Some("/c".into()));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn empty_history_goes_nowhere() {
        let mut h = History::default();
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
        assert!(h.recent().is_empty());
    }

    #[test]
    fn visiting_the_current_entry_is_a_no_op() {
        // back() moves first; the Listed that follows visits the same path.
        let mut h = h(&["/a", "/b"]);
        assert_eq!(h.back(), Some("/a".into()));
        h.visit(Path::new("/a"));
        assert_eq!(h.forward(), Some("/b".into()));
    }

    #[test]
    fn new_visit_after_back_drops_forward() {
        let mut h = h(&["/a", "/b", "/c"]);
        h.back();
        h.visit(Path::new("/x"));
        assert_eq!(h.forward(), None);
        assert_eq!(h.back(), Some("/b".into()));
    }

    #[test]
    fn limit_drops_oldest_keeps_position() {
        let paths: Vec<String> = (0..=LIMIT).map(|i| format!("/d{i}")).collect();
        let mut h = History::default();
        for p in &paths {
            h.visit(Path::new(p));
        }
        let mut steps = 0;
        while h.back().is_some() {
            steps += 1;
        }
        assert_eq!(steps, LIMIT - 1); // LIMIT entries kept, oldest (/d0) dropped
        assert_eq!(h.forward(), Some("/d2".into()));
    }

    #[test]
    fn regression_recent_starts_with_the_current_entry_after_back() {
        let mut h = h(&["/a", "/b", "/c"]);
        h.back(); // now at /b
        assert_eq!(h.recent(), [PathBuf::from("/b"), "/c".into(), "/a".into()]);
    }

    #[test]
    fn recent_is_unique_newest_first() {
        let h = h(&["/a", "/b", "/a", "/c"]);
        assert_eq!(h.recent(), [PathBuf::from("/c"), "/a".into(), "/b".into()]);
    }

    #[test]
    fn regression_failed_step_then_enter_keeps_the_shown_dir() {
        let mut h = History::default();
        for p in ["/a", "/b"] {
            h.visit(Path::new(p));
        }
        assert_eq!(h.back(), Some("/a".into())); // /a was deleted: the listing fails
        h.seat(Path::new("/b")); // still on /b
        h.visit(Path::new("/b/sub")); // Enter
        assert_eq!(
            h.recent(),
            [PathBuf::from("/b/sub"), "/b".into(), "/a".into()]
        );
        assert_eq!(h.back(), Some("/b".into()));
    }

    #[test]
    fn regression_a_saved_pos_past_the_end_does_not_panic() {
        let mut h = History {
            items: vec!["/a".into(), "/b".into()],
            pos: 5,
        };
        assert_eq!(h.back(), Some(PathBuf::from("/a")));
        assert_eq!(h.back(), None);
    }
}
