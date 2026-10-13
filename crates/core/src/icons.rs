//! Icon names a toolbar button can use: the symbolic icons of the given themes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Names of `*-symbolic` icons (svg / png) under `<root>/<theme>/` for every root and theme,
/// sorted, each once. Missing dirs are skipped.
pub fn symbolic(roots: &[PathBuf], themes: &[&str]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for root in roots {
        for theme in themes {
            walk(&root.join(theme), &mut names);
        }
    }
    names.into_iter().collect()
}

/// An icon theme `name` under `<data dir>/icons/` (`openwith::data_dirs`).
pub fn theme_installed(name: &str, data_dirs: &[PathBuf]) -> bool {
    data_dirs
        .iter()
        .any(|d| d.join("icons").join(name).join("index.theme").is_file())
}

/// A theme name as the desktop's tools print it: `Papirus` (xfconf-query), `'Adwaita'` (gsettings).
pub fn theme_name(out: &str) -> Option<String> {
    let t = out.trim().trim_matches('\'');
    (!t.is_empty()).then(|| t.to_string())
}

/// `gtk-icon-theme-name=…` of a GTK `settings.ini`.
pub fn ini_theme(ini: &str) -> Option<String> {
    ini.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "gtk-icon-theme-name")
        .and_then(|(_, v)| theme_name(v.trim().trim_matches('"')))
}

fn walk(dir: &Path, names: &mut BTreeSet<String>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        // file_type: a theme's symlinked dirs (`scalable -> ...`) are not followed, no loops.
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&p, names);
        } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            && stem.ends_with("-symbolic")
            && p.extension().is_some_and(|x| x == "svg" || x == "png")
        {
            names.insert(stem.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbolic_icons_of_the_themes_once_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let file = |p: &str| {
            let p = tmp.path().join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "").unwrap();
        };
        file("a/Cosmic/scalable/actions/edit-copy-symbolic.svg");
        file("a/Cosmic/scalable/actions/edit-copy.svg"); // not symbolic
        file("a/Other/x/zzz-symbolic.svg"); // other theme
        file("b/Adwaita/16x16/go-up-symbolic.png");
        file("b/Adwaita/symbolic/edit-copy-symbolic.svg"); // twice
        file("b/Adwaita/symbolic/readme-symbolic.txt");
        let roots = [
            tmp.path().join("a"),
            tmp.path().join("b"),
            tmp.path().join("none"),
        ];
        assert_eq!(
            symbolic(&roots, &["Cosmic", "Adwaita"]),
            ["edit-copy-symbolic", "go-up-symbolic"]
        );
    }

    #[test]
    fn desktop_icon_theme_names() {
        assert_eq!(theme_name("Papirus\n").as_deref(), Some("Papirus"));
        assert_eq!(theme_name("'Adwaita'\n").as_deref(), Some("Adwaita"));
        assert_eq!(theme_name("  \n"), None);
        let ini = "[Settings]\ngtk-theme-name=Materia\ngtk-icon-theme-name = \"Papirus-Dark\"\n";
        assert_eq!(ini_theme(ini).as_deref(), Some("Papirus-Dark"));
        assert_eq!(ini_theme("[Settings]\n"), None);
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("icons/Papirus")).unwrap();
        std::fs::write(d.path().join("icons/Papirus/index.theme"), "").unwrap();
        let dirs = [d.path().to_path_buf()];
        assert!(theme_installed("Papirus", &dirs) && !theme_installed("Cosmic", &dirs));
    }
}
