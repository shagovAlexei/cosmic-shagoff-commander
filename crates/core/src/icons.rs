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
}
