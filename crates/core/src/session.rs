//! Remembered tabs: what is saved per pane and how it is restored when dirs have gone.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneState {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
}

/// `path` or its nearest existing ancestor dir; `fallback` for relative or empty paths.
pub fn existing_dir(path: &Path, fallback: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| p.is_absolute() && p.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// Tab paths to open and the active index; never empty.
pub fn restore(state: &PaneState, fallback: &Path) -> (Vec<PathBuf>, usize) {
    if state.tabs.is_empty() {
        return (vec![fallback.to_path_buf()], 0);
    }
    let tabs: Vec<PathBuf> = state
        .tabs
        .iter()
        .map(|p| existing_dir(p, fallback))
        .collect();
    let active = state.active.min(tabs.len() - 1);
    (tabs, active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_dir_walks_up_to_a_live_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("a/b/c");
        assert_eq!(existing_dir(&gone, Path::new("/fallback")), tmp.path());
        assert_eq!(existing_dir(tmp.path(), Path::new("/fallback")), tmp.path());
    }

    #[test]
    fn existing_dir_skips_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(existing_dir(&file, Path::new("/fallback")), tmp.path());
    }

    #[test]
    fn existing_dir_relative_or_empty_gives_fallback() {
        assert_eq!(
            existing_dir(Path::new("rel/dir"), Path::new("/fb")),
            Path::new("/fb")
        );
        assert_eq!(
            existing_dir(Path::new(""), Path::new("/fb")),
            Path::new("/fb")
        );
    }

    #[test]
    fn restore_empty_state_opens_fallback() {
        let (tabs, active) = restore(&PaneState::default(), Path::new("/fb"));
        assert_eq!(tabs, [PathBuf::from("/fb")]);
        assert_eq!(active, 0);
    }

    #[test]
    fn restore_clamps_active_and_fixes_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let state = PaneState {
            tabs: vec![tmp.path().into(), tmp.path().join("gone"), "rel".into()],
            active: 7,
        };
        let (tabs, active) = restore(&state, Path::new("/fb"));
        assert_eq!(
            tabs,
            [tmp.path().to_path_buf(), tmp.path().into(), "/fb".into()]
        );
        assert_eq!(active, 2);
    }
}
