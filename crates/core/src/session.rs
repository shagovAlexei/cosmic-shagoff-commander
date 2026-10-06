//! Remembered tabs: what is saved per pane and how it is restored when dirs have gone.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneState {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
    /// Per tab, by index: locked (TC), own caption ("" = the dir's name). Missing in old state.
    #[serde(default)]
    pub locked: Vec<bool>,
    #[serde(default)]
    pub names: Vec<String>,
    /// Per tab: Brief view (Ctrl+F1) instead of Full.
    #[serde(default)]
    pub brief: Vec<bool>,
    /// Per tab: dir history (Alt+← / → / ↓); empty when "Remember history" is off.
    #[serde(default)]
    pub history: Vec<crate::history::History>,
}

/// `~` and `~/x` → under `home`; anything else unchanged (`~user` is not supported). A trailing
/// `/` stays: F5 reads it as "into this dir".
pub fn expand_home(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) if path.as_os_str().as_encoded_bytes().ends_with(b"/") => {
            let mut s = home.join(rest).into_os_string();
            if !s.as_encoded_bytes().ends_with(b"/") {
                s.push("/");
            }
            s.into()
        }
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// `path` or its nearest existing ancestor dir; `fallback` for relative or empty paths.
// ponytail: `is_dir` blocks on a dead network mount at startup; restore in spawn_blocking if that bites.
pub fn existing_dir(path: &Path, fallback: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| p.is_absolute() && (p.is_dir() || crate::archive::split_path(p).is_some()))
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
    fn expand_home_handles_tilde_forms() {
        let home = Path::new("/home/u");
        assert_eq!(expand_home(Path::new("~"), home), home);
        assert_eq!(
            expand_home(Path::new("~/work"), home),
            Path::new("/home/u/work")
        );
        assert_eq!(expand_home(Path::new("/tmp"), home), Path::new("/tmp"));
        // Path equality ignores a trailing slash: compare the strings.
        let s = |p: &str| expand_home(Path::new(p), home).into_os_string();
        assert_eq!(s("~/new/"), "/home/u/new/");
        assert_eq!(s("~/"), "/home/u/");
        assert_eq!(
            expand_home(Path::new("~other/x"), home),
            Path::new("~other/x")
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
            ..PaneState::default()
        };
        let (tabs, active) = restore(&state, Path::new("/fb"));
        assert_eq!(
            tabs,
            [tmp.path().to_path_buf(), tmp.path().into(), "/fb".into()]
        );
        assert_eq!(active, 2);
    }

    #[test]
    fn existing_dir_inside_archive() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
        z.start_file("d/f", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        let fb = Path::new("/");
        assert_eq!(existing_dir(&a.join("d"), fb), a.join("d"));
        std::fs::remove_file(&a).unwrap();
        assert_eq!(existing_dir(&a.join("d"), fb), d.path());
    }
}
