//! Commands → «Space map»: what a dir's contents take, biggest first, each dir with its own
//! contents to step into. Read once, off the UI thread; stepping in and out reads nothing.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Rows kept by name in each dir, the biggest; the rest are one «the rest» row.
pub const ROWS_KEPT: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Empty for the «the rest» row.
    pub name: OsString,
    /// Bytes on disk under it (a link: its own, never followed; a hard-linked file once).
    pub size: u64,
    pub dir: bool,
    /// Files under it.
    pub files: u64,
    /// Something in it could not be read: its size is short of the real one.
    pub unread: bool,
    /// A dir: its contents, biggest first. Empty for a dir on another filesystem (not read,
    /// as `du -x`).
    pub children: Vec<Node>,
}

impl Node {
    /// The node `at` (child indexes) steps down to; `None` when that is gone.
    pub fn at(&self, at: &[usize]) -> Option<&Node> {
        at.iter().try_fold(self, |n, &i| n.children.get(i))
    }
}

/// The map on screen: the dir shown (`at`, child indexes from the root) and the cursor in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    pub root: PathBuf,
    pub tree: Node,
    pub at: Vec<usize>,
    pub cursor: usize,
}

impl Map {
    pub fn new(root: PathBuf, tree: Node) -> Self {
        Map {
            root,
            tree,
            at: Vec::new(),
            cursor: 0,
        }
    }

    /// The dir whose contents are shown.
    pub fn shown(&self) -> &Node {
        self.tree.at(&self.at).unwrap_or(&self.tree)
    }

    /// Its path.
    pub fn dir(&self) -> PathBuf {
        let mut n = &self.tree;
        let mut p = self.root.clone();
        for &i in &self.at {
            n = &n.children[i];
            p.push(&n.name);
        }
        p
    }

    pub fn current(&self) -> Option<&Node> {
        self.shown().children.get(self.cursor)
    }

    pub fn move_cursor(&mut self, by: isize) {
        let last = self.shown().children.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Into the dir under the cursor (one with contents read); `false`: not a dir.
    pub fn step_in(&mut self) -> bool {
        if !self
            .current()
            .is_some_and(|n| n.dir && !n.children.is_empty())
        {
            return false;
        }
        self.at.push(self.cursor);
        self.cursor = 0;
        true
    }

    /// Back up, the cursor on the dir left; `false` at the root.
    pub fn step_out(&mut self) -> bool {
        match self.at.pop() {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }
}

/// `root` read whole (a link to a dir: the dir), not leaving its filesystem. `None` = stopped.
pub fn scan(root: &Path, stop: &AtomicBool) -> Option<Node> {
    let dev = fs::metadata(root).map(|m| m.dev()).unwrap_or(0);
    let name = root
        .file_name()
        .map_or_else(|| root.as_os_str().to_owned(), |n| n.to_owned());
    let mut w = Walk {
        dev,
        stop,
        seen: HashSet::new(),
    };
    w.dir(root, name)
}

struct Walk<'a> {
    dev: u64,
    stop: &'a AtomicBool,
    /// (dev, inode) of files with more than one link: each counted once.
    seen: HashSet<(u64, u64)>,
}

impl Walk<'_> {
    // ponytail: recursive — a tree nested thousands deep would run out of stack; make it a loop
    // if one ever shows up.
    fn dir(&mut self, path: &Path, name: OsString) -> Option<Node> {
        if self.stop.load(Ordering::Relaxed) {
            return None;
        }
        let mut node = Node {
            name,
            dir: true,
            ..Node::default()
        };
        let Ok(rd) = fs::read_dir(path) else {
            node.unread = true;
            return Some(node);
        };
        for e in rd {
            let Ok(e) = e else {
                node.unread = true;
                continue;
            };
            let name = e.file_name();
            let Ok(m) = e.metadata() else {
                node.unread = true; // not followed: a link's own
                continue;
            };
            let child = if m.is_dir() && m.dev() == self.dev {
                self.dir(&e.path(), name)?
            } else if m.is_dir() {
                // Another filesystem: not read (`du -x`).
                Node {
                    name,
                    dir: true,
                    ..Node::default()
                }
            } else {
                let again = m.nlink() > 1 && !self.seen.insert((m.dev(), m.ino()));
                Node {
                    name,
                    // On disk, not the length: a sparse file takes what it holds.
                    size: if again { 0 } else { m.blocks() * 512 },
                    files: 1,
                    ..Node::default()
                }
            };
            node.unread |= child.unread;
            node.children.push(child);
        }
        node.size = node.children.iter().map(|c| c.size).sum();
        node.files = node.children.iter().map(|c| c.files).sum();
        node.children.sort_by_key(|c| std::cmp::Reverse(c.size));
        if node.children.len() > ROWS_KEPT {
            let rest = node.children.split_off(ROWS_KEPT);
            node.children.push(Node {
                size: rest.iter().map(|c| c.size).sum(),
                files: rest.iter().map(|c| c.files).sum(),
                ..Node::default()
            });
        }
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dir_is_summed_biggest_first_and_steps_in() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("big/sub")).unwrap();
        fs::write(t.path().join("big/sub/x"), vec![1; 300_000]).unwrap();
        fs::write(t.path().join("big/y"), vec![1; 100_000]).unwrap();
        fs::write(t.path().join("small"), vec![0; 50]).unwrap();
        std::os::unix::fs::symlink("/usr", t.path().join("link")).unwrap(); // not followed
        // A hard link: its file counted once.
        fs::hard_link(t.path().join("small"), t.path().join("small2")).unwrap();
        let n = scan(t.path(), &AtomicBool::new(false)).unwrap();
        assert!(n.dir);
        let names: Vec<_> = n
            .children
            .iter()
            .map(|c| c.name.to_str().unwrap())
            .collect();
        assert_eq!(&names[..1], ["big"]);
        let big = &n.children[0];
        assert_eq!(big.size, big.children.iter().map(|c| c.size).sum::<u64>());
        assert_eq!(n.files, 5);
        let x = n.children.iter().find(|c| c.name == "small").unwrap().size;
        let twin = n.children.iter().find(|c| c.name == "small2").unwrap().size;
        assert_eq!(x + twin, x.max(twin)); // one of the two is 0
        assert!(!n.unread);
        assert_eq!(n.at(&[0, 0]).map(|s| s.name.to_str()), Some(Some("sub")));
        assert_eq!(n.at(&[0, 0, 0]).map(|s| s.files), Some(1));
        assert_eq!(n.at(&[9]), None);
        // Stopped: nothing.
        assert_eq!(scan(t.path(), &AtomicBool::new(true)), None);
    }

    #[test]
    fn the_map_steps_in_and_back_out_onto_the_dir_left() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("a/b")).unwrap();
        fs::write(t.path().join("a/b/x"), vec![0; 10]).unwrap();
        fs::write(t.path().join("z"), vec![0; 5]).unwrap();
        fs::create_dir(t.path().join("empty")).unwrap();
        let tree = scan(t.path(), &AtomicBool::new(false)).unwrap();
        let mut m = Map::new(t.path().to_path_buf(), tree);
        m.move_cursor(1);
        assert!(!m.step_in()); // a file
        m.move_cursor(5);
        assert_eq!(m.current().map(|n| n.name.to_str()), Some(Some("empty")));
        assert!(!m.step_in()); // nothing in it
        m.move_cursor(-9);
        assert!(m.step_in() && m.step_in());
        assert_eq!(m.dir(), t.path().join("a/b"));
        assert_eq!(m.current().map(|n| n.name.to_str()), Some(Some("x")));
        assert!(m.step_out());
        assert_eq!(m.current().map(|n| n.name.to_str()), Some(Some("b")));
        assert!(m.step_out() && !m.step_out());
        assert_eq!((m.dir(), m.cursor), (t.path().to_path_buf(), 0));
    }

    #[test]
    fn many_entries_are_one_row_for_the_rest() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..ROWS_KEPT + 5 {
            fs::write(t.path().join(format!("f{i}")), "x").unwrap();
        }
        let n = scan(t.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(n.children.len(), ROWS_KEPT + 1);
        let rest = n.children.last().unwrap();
        assert!(rest.name.is_empty() && rest.files == 5);
        assert_eq!(n.files, (ROWS_KEPT + 5) as u64);
    }
}
