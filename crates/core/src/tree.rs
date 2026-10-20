//! Alt+F10 (TC «Change directory via tree»): the dirs as an outline. A flat list of the rows on
//! screen — a node's children right after it, one level deeper — opened lazily: the UI reads a
//! dir's subdirs off its thread (`subdirs`) and hands them to `fill`.

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: PathBuf,
    /// The last component (`/` for the root).
    pub name: String,
    pub depth: usize,
    /// Its children are the rows after it.
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub rows: Vec<Node>,
    pub cursor: usize,
}

impl Tree {
    /// Just `root`, closed.
    pub fn new(root: &Path) -> Self {
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Tree {
            rows: vec![Node {
                path: root.to_path_buf(),
                name,
                depth: 0,
                open: false,
            }],
            cursor: 0,
        }
    }

    pub fn current(&self) -> Option<&Node> {
        self.rows.get(self.cursor)
    }

    /// `dir`'s subdirs read: shown under it, sorted (case aside), `dir` open. A dir not on screen
    /// (closed meanwhile, or never shown): nothing.
    pub fn fill(&mut self, dir: &Path, mut names: Vec<String>) {
        let Some(i) = self.rows.iter().position(|n| n.path == dir) else {
            return;
        };
        self.close(i);
        names.sort_by_key(|n| n.to_lowercase());
        let depth = self.rows[i].depth + 1;
        let kids = names.into_iter().map(|name| Node {
            path: dir.join(&name),
            name,
            depth,
            open: false,
        });
        self.rows.splice(i + 1..i + 1, kids);
        self.rows[i].open = true;
    }

    /// Row `i`'s children off screen; the cursor, if among them, on `i`.
    pub fn close(&mut self, i: usize) {
        let Some(n) = self.rows.get(i) else { return };
        let depth = n.depth;
        let end = (i + 1..self.rows.len())
            .find(|&j| self.rows[j].depth <= depth)
            .unwrap_or(self.rows.len());
        if self.cursor > i && self.cursor < end {
            self.cursor = i;
        } else if self.cursor >= end {
            self.cursor -= end - i - 1;
        }
        self.rows.drain(i + 1..end);
        self.rows[i].open = false;
    }

    /// The row of `i`'s parent dir.
    pub fn parent(&self, i: usize) -> Option<usize> {
        let depth = self.rows.get(i)?.depth;
        (0..i).rev().find(|&j| self.rows[j].depth < depth)
    }

    /// The cursor on `path`'s row, if shown.
    pub fn select(&mut self, path: &Path) {
        if let Some(i) = self.rows.iter().position(|n| n.path == path) {
            self.cursor = i;
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }
}

/// The dirs to open, from `root` down, for `target` to be shown: its ancestors below `root`'s
/// level and `root` itself — not `target` (TC shows it closed).
pub fn chain(root: &Path, target: &Path) -> Vec<PathBuf> {
    let Ok(rest) = target.strip_prefix(root) else {
        return Vec::new();
    };
    let mut out = vec![root.to_path_buf()];
    let mut at = root.to_path_buf();
    let parts: Vec<_> = rest.components().collect();
    for c in parts.iter().take(parts.len().saturating_sub(1)) {
        at.push(c);
        out.push(at.clone());
    }
    out
}

/// The subdirs of `dir` by name (links to dirs too); hidden ones only with `hidden`. Unreadable
/// entries are left out.
pub fn subdirs(dir: &Path, hidden: bool) -> io::Result<Vec<String>> {
    Ok(std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                || e.file_type().is_ok_and(|t| t.is_symlink()) && e.path().is_dir()
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| hidden || !n.starts_with('.'))
        .collect())
}

/// `subdirs`, and the one on the way to `toward` even if hidden (the tree must reach the dir it
/// was opened in, say `~/.config/x`).
pub fn subdirs_toward(dir: &Path, hidden: bool, toward: Option<&Path>) -> io::Result<Vec<String>> {
    let mut names = subdirs(dir, hidden)?;
    let next = toward
        .and_then(|t| t.strip_prefix(dir).ok())
        .and_then(|rest| rest.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    if let Some(n) = next.filter(|n| !names.contains(n)) {
        names.push(n);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(t: &Tree) -> Vec<String> {
        t.rows
            .iter()
            .map(|n| format!("{}{}", " ".repeat(n.depth), n.name))
            .collect()
    }

    #[test]
    fn opens_the_way_down_and_closes() {
        let mut t = Tree::new(Path::new("/"));
        assert_eq!(names(&t), ["/"]);
        t.fill(
            Path::new("/"),
            vec!["usr".into(), "Home".into(), "etc".into()],
        );
        assert_eq!(names(&t), ["/", " etc", " Home", " usr"]); // case aside
        t.fill(Path::new("/usr"), vec!["lib".into(), "bin".into()]);
        assert_eq!(names(&t), ["/", " etc", " Home", " usr", "  bin", "  lib"]);
        t.select(Path::new("/usr/lib"));
        assert_eq!((t.cursor, t.parent(t.cursor)), (5, Some(3)));
        t.close(3); // the cursor inside: onto the closed one
        assert_eq!((names(&t).len(), t.cursor), (4, 3));
        assert!(!t.rows[3].open);
        // Closing above the cursor keeps it on its row.
        t.fill(Path::new("/etc"), vec!["x".into()]);
        t.select(Path::new("/usr"));
        t.close(1);
        assert_eq!(t.current().unwrap().path, Path::new("/usr"));
        // A dir not shown: nothing.
        t.fill(Path::new("/nope"), vec!["y".into()]);
        assert_eq!(names(&t).len(), 4);
        t.move_cursor(-10);
        assert_eq!(t.cursor, 0);
        t.move_cursor(10);
        assert_eq!(t.cursor, 3);
    }

    #[test]
    fn chain_and_subdirs() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            chain(&p("/"), &p("/home/u/src")),
            [p("/"), p("/home"), p("/home/u")]
        );
        assert_eq!(chain(&p("/"), &p("/")), [p("/")]);
        assert!(chain(&p("/a"), &p("/b")).is_empty());
        let d = tempfile::tempdir().unwrap();
        for n in ["b", "a", ".hid"] {
            std::fs::create_dir(d.path().join(n)).unwrap();
        }
        std::fs::write(d.path().join("file"), "").unwrap();
        std::os::unix::fs::symlink(d.path().join("a"), d.path().join("ln")).unwrap();
        let mut got = subdirs(d.path(), false).unwrap();
        got.sort();
        assert_eq!(got, ["a", "b", "ln"]);
        assert_eq!(subdirs(d.path(), true).unwrap().len(), 4);
        // The way to a hidden dir is shown all the same.
        let toward = d.path().join(".hid/deeper");
        let got = subdirs_toward(d.path(), false, Some(&toward)).unwrap();
        assert!(got.contains(&".hid".to_string()) && got.len() == 4);
    }
}
