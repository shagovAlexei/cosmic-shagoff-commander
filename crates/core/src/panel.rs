use crate::listing::{Entry, Kind};
use crate::mask::Mask;
use crate::sort::{Sort, SortKey, sort_entries};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Name of the synthetic "go up" row.
pub const PARENT: &str = "..";

/// Status-line totals, excluding the `..` row.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub bytes: u64,
    pub files: usize,
    pub dirs: usize,
}

/// One panel's state. Never touches the filesystem: the UI scans and hands results to `set_listing`.
#[derive(Debug, Clone)]
pub struct Panel {
    cwd: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    sort: Sort,
    show_hidden: bool,
    /// Marked entries by real name, so marks survive re-sorting and rescans.
    marked: HashSet<OsString>,
}

impl Panel {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            cursor: 0,
            sort: Sort::default(),
            show_hidden: false,
            marked: HashSet::new(),
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Only the flag: rescan the dir afterwards for the listing to change.
    pub fn set_show_hidden(&mut self, on: bool) {
        self.show_hidden = on;
    }

    /// The only way to load a directory, so cwd, entries and cursor always change together.
    /// Cursor: `focus` if present; else on a rescan of the same dir the same name, falling back to the old index;
    /// else (new dir) the first row. Always clamped.
    pub fn set_listing(&mut self, cwd: PathBuf, mut entries: Vec<Entry>, focus: Option<&str>) {
        let same_dir = cwd == self.cwd;
        let keep = match focus {
            Some(f) => Some(f.to_owned()),
            None if same_dir => self.current().map(|e| e.name.clone()),
            None => None,
        };
        let old = if same_dir { self.cursor } else { 0 };
        sort_entries(&mut entries, self.sort);
        if same_dir {
            let present: HashSet<&OsString> = entries.iter().map(|e| &e.os_name).collect();
            self.marked.retain(|k| present.contains(k));
        } else {
            self.marked.clear();
        }
        if cwd.parent().is_some() {
            entries.insert(0, parent_entry());
        }
        self.cwd = cwd;
        self.entries = entries;
        self.cursor = keep.and_then(|n| self.index_of(&n)).unwrap_or(old);
        self.move_cursor(0); // clamp
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    pub fn cursor_end(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
    }

    /// Same column flips direction; a new column starts ascending. Keeps `..` first and the cursor on its name.
    pub fn set_sort(&mut self, key: SortKey) {
        let asc = self.sort.key != key || !self.sort.asc;
        self.sort = Sort { key, asc };
        let name = self.current().map(|e| e.name.clone());
        let start = usize::from(self.parent_row());
        sort_entries(&mut self.entries[start..], self.sort);
        if let Some(i) = name.and_then(|n| self.index_of(&n)) {
            self.cursor = i;
        }
    }

    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
        self.move_cursor(0); // clamp
    }

    pub fn totals(&self) -> Totals {
        let start = usize::from(self.parent_row());
        sum(self.entries[start..].iter())
    }

    pub fn is_marked(&self, e: &Entry) -> bool {
        self.marked.contains(&e.os_name)
    }

    /// Space: flip the mark of the row under the cursor (never `..`).
    pub fn toggle_mark(&mut self) {
        let Some(e) = self.current() else { return };
        if e.name == PARENT {
            return;
        }
        let key = e.os_name.clone();
        if !self.marked.remove(&key) {
            self.marked.insert(key);
        }
    }

    /// Insert / Shift+↓ (+1), Shift+↑ (−1).
    pub fn toggle_mark_and_move(&mut self, delta: isize) {
        self.toggle_mark();
        self.move_cursor(delta);
    }

    /// Num+ / Num−: files only.
    pub fn mark_by_mask(&mut self, mask: &Mask, on: bool) {
        for e in self
            .entries
            .iter()
            .filter(|e| !e.is_dir() && mask.matches(&e.name))
        {
            if on {
                self.marked.insert(e.os_name.clone());
            } else {
                self.marked.remove(&e.os_name);
            }
        }
    }

    /// Num*: files only.
    pub fn invert(&mut self) {
        for e in self.entries.iter().filter(|e| !e.is_dir()) {
            if !self.marked.remove(&e.os_name) {
                self.marked.insert(e.os_name.clone());
            }
        }
    }

    /// Ctrl+A / Ctrl+Num−: everything except `..`.
    pub fn mark_all(&mut self, on: bool) {
        self.marked.clear();
        if on {
            let start = usize::from(self.parent_row());
            self.marked
                .extend(self.entries[start..].iter().map(|e| e.os_name.clone()));
        }
    }

    pub fn marked_totals(&self) -> Totals {
        sum(self.entries.iter().filter(|e| self.is_marked(e)))
    }

    /// What an operation acts on: marked entries (in list order), else the row under the cursor; never `..`.
    pub fn targets(&self) -> Vec<PathBuf> {
        let marked: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|e| self.is_marked(e))
            .map(|e| self.cwd.join(&e.os_name))
            .collect();
        if !marked.is_empty() {
            return marked;
        }
        self.current()
            .filter(|e| e.name != PARENT)
            .map(|e| vec![self.cwd.join(&e.os_name)])
            .unwrap_or_default()
    }

    /// Drop marks of processed entries (by file name).
    pub fn unmark(&mut self, paths: &[PathBuf]) {
        for name in paths.iter().filter_map(|p| p.file_name()) {
            self.marked.remove(name);
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// Parent dir and the name to focus there (the dir we leave).
    pub fn parent_path(&self) -> Option<(PathBuf, String)> {
        parent_of(&self.cwd)
    }

    /// Where Enter leads: `..` → parent (with focus), a dir → inside it, a file → `None`.
    pub fn enter_path(&self) -> Option<(PathBuf, Option<String>)> {
        let e = self.current()?;
        if e.name == PARENT {
            return self.parent_path().map(|(p, n)| (p, Some(n)));
        }
        e.is_dir().then(|| (self.cwd.join(&e.os_name), None))
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    fn parent_row(&self) -> bool {
        self.entries.first().is_some_and(|e| e.name == PARENT)
    }
}

fn sum<'a>(entries: impl Iterator<Item = &'a Entry>) -> Totals {
    entries.fold(Totals::default(), |mut t, e| {
        if e.is_dir() {
            t.dirs += 1;
        } else {
            t.files += 1;
            t.bytes += e.size;
        }
        t
    })
}

fn parent_entry() -> Entry {
    Entry {
        name: PARENT.into(),
        os_name: PARENT.into(),
        ext: String::new(),
        size: 0,
        mtime: UNIX_EPOCH,
        kind: Kind::Dir,
        is_link: false,
        mode: 0,
    }
}

/// `/a/b` → (`/a`, "b"): where Backspace leads and which name to put the cursor on.
pub fn parent_of(path: &Path) -> Option<(PathBuf, String)> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some((parent.to_path_buf(), name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn f(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: String::new(),
            size,
            mtime: UNIX_EPOCH,
            kind: Kind::File,
            is_link: false,
            mode: 0,
        }
    }
    fn d(name: &str) -> Entry {
        Entry {
            kind: Kind::Dir,
            ..f(name, 0)
        }
    }
    fn names(p: &Panel) -> Vec<&str> {
        p.entries().iter().map(|e| e.name.as_str()).collect()
    }
    fn loaded(cwd: &str, entries: Vec<Entry>) -> Panel {
        let mut p = Panel::new(PathBuf::from(cwd));
        p.set_listing(PathBuf::from(cwd), entries, None);
        p
    }

    #[test]
    fn parent_row_first_except_at_root() {
        let p = loaded("/home/u", vec![f("b", 1), d("a")]);
        assert_eq!(names(&p), ["..", "a", "b"]);
        let root = loaded("/", vec![f("b", 1), d("a")]);
        assert_eq!(names(&root), ["a", "b"]);
    }

    #[test]
    fn empty_root_has_no_rows_and_cursor_is_safe() {
        let mut p = loaded("/", vec![]);
        assert!(p.current().is_none());
        p.move_cursor(5);
        p.move_cursor(-5);
        p.cursor_end();
        assert_eq!(p.cursor(), 0);
        assert!(p.enter_path().is_none());
    }

    #[test]
    fn cursor_clamps_at_edges() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1), f("c", 1)]);
        p.move_cursor(-1);
        assert_eq!(p.cursor(), 0);
        p.move_cursor(100);
        assert_eq!(p.cursor(), 3);
        p.cursor_home();
        assert_eq!(p.cursor(), 0);
        p.cursor_end();
        assert_eq!(p.current().unwrap().name, "c");
    }

    #[test]
    fn new_dir_puts_cursor_on_first_row() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1), f("c", 1)]);
        p.cursor_end();
        p.set_listing(
            PathBuf::from("/x/y"),
            vec![f("q", 1), f("r", 1), f("s", 1)],
            None,
        );
        assert_eq!(p.cursor(), 0);
        assert_eq!(p.cwd(), Path::new("/x/y"));
    }

    #[test]
    fn rescan_keeps_cursor_on_same_name() {
        let mut p = loaded("/x", vec![f("b", 1), f("c", 1)]);
        p.cursor_end(); // on "c"
        p.set_listing(
            PathBuf::from("/x"),
            vec![f("a", 1), f("b", 1), f("c", 1)],
            None,
        );
        assert_eq!(p.current().unwrap().name, "c");
    }

    #[test]
    fn rescan_keeps_index_when_name_gone() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1), f("c", 1)]);
        p.move_cursor(2); // on "b" (index 2, after "..")
        p.set_listing(PathBuf::from("/x"), vec![f("a", 1), f("c", 1)], None);
        assert_eq!(p.current().unwrap().name, "c");
    }

    #[test]
    fn rescan_clamps_when_list_shrinks() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1), f("c", 1)]);
        p.cursor_end();
        p.set_listing(PathBuf::from("/x"), vec![f("a", 1)], None);
        assert_eq!(p.current().unwrap().name, "a");
    }

    #[test]
    fn focus_wins_over_everything() {
        let mut p = loaded("/x/y", vec![f("q", 1)]);
        p.set_listing(PathBuf::from("/x"), vec![d("w"), d("y"), d("z")], Some("y"));
        assert_eq!(p.current().unwrap().name, "y");
    }

    #[test]
    fn set_sort_toggles_and_keeps_cursor_and_parent_row() {
        let mut p = loaded("/x", vec![f("a", 3), f("b", 1), f("c", 2)]);
        p.move_cursor(1); // on "a"
        p.set_sort(SortKey::Size);
        assert_eq!(
            p.sort(),
            Sort {
                key: SortKey::Size,
                asc: true
            }
        );
        assert_eq!(names(&p), ["..", "b", "c", "a"]);
        assert_eq!(p.current().unwrap().name, "a");
        p.set_sort(SortKey::Size);
        assert_eq!(
            p.sort(),
            Sort {
                key: SortKey::Size,
                asc: false
            }
        );
        assert_eq!(names(&p), ["..", "a", "c", "b"]);
        assert_eq!(p.current().unwrap().name, "a");
    }

    #[test]
    fn set_listing_uses_current_sort() {
        let mut p = loaded("/x", vec![]);
        p.set_sort(SortKey::Date);
        let mut old = f("old", 1);
        old.mtime = UNIX_EPOCH + Duration::from_secs(1);
        let mut new = f("new", 1);
        new.mtime = UNIX_EPOCH + Duration::from_secs(2);
        p.set_listing(PathBuf::from("/x"), vec![new, old], None);
        assert_eq!(names(&p), ["..", "old", "new"]);
    }

    #[test]
    fn enter_non_utf8_dir_uses_real_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let mut bad = d("ignored");
        bad.os_name = OsStr::from_bytes(b"x\xff").to_owned();
        bad.name = bad.os_name.to_string_lossy().into_owned();
        let mut p = loaded("/x", vec![bad]);
        p.move_cursor(1);
        assert_eq!(
            p.enter_path(),
            Some((Path::new("/x").join(OsStr::from_bytes(b"x\xff")), None))
        );
    }

    #[test]
    fn set_cursor_clamps() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
        p.set_cursor(1);
        assert_eq!(p.current().unwrap().name, "a");
        p.set_cursor(99);
        assert_eq!(p.current().unwrap().name, "b");
    }

    #[test]
    fn totals_skip_parent_row() {
        let p = loaded("/x", vec![d("sub"), f("a", 1000), f("b", 24)]);
        assert_eq!(
            p.totals(),
            Totals {
                bytes: 1024,
                files: 2,
                dirs: 1
            }
        );
        assert_eq!(loaded("/", vec![]).totals(), Totals::default());
    }

    fn marked_names(p: &Panel) -> Vec<&str> {
        p.entries()
            .iter()
            .filter(|e| p.is_marked(e))
            .map(|e| e.name.as_str())
            .collect()
    }

    #[test]
    fn parent_row_is_never_marked() {
        let mut p = loaded("/x", vec![f("a", 1)]);
        p.toggle_mark(); // cursor on ".."
        p.mark_all(true);
        assert_eq!(marked_names(&p), ["a"]);
    }

    #[test]
    fn insert_marks_and_moves_down_shift_up_moves_up() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1), f("c", 1)]);
        p.set_cursor(1);
        p.toggle_mark_and_move(1);
        assert_eq!(p.current().unwrap().name, "b");
        p.toggle_mark_and_move(1);
        assert_eq!(marked_names(&p), ["a", "b"]);
        p.toggle_mark_and_move(-1); // toggles "c", back to "b"
        assert_eq!(p.current().unwrap().name, "b");
        assert_eq!(marked_names(&p), ["a", "b", "c"]);
        p.toggle_mark(); // Space: unmark "b", cursor stays
        assert_eq!(marked_names(&p), ["a", "c"]);
        assert_eq!(p.current().unwrap().name, "b");
    }

    #[test]
    fn marks_survive_sort() {
        let mut p = loaded("/x", vec![f("a", 3), f("b", 1)]);
        p.set_cursor(1);
        p.toggle_mark();
        p.set_sort(SortKey::Size);
        assert_eq!(marked_names(&p), ["a"]);
    }

    #[test]
    fn rescan_prunes_vanished_marks() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
        p.mark_all(true);
        p.set_listing(PathBuf::from("/x"), vec![f("b", 1), f("c", 1)], None);
        assert_eq!(marked_names(&p), ["b"]);
        // "a" coming back later must not be resurrected as marked
        p.set_listing(PathBuf::from("/x"), vec![f("a", 1), f("b", 1)], None);
        assert_eq!(marked_names(&p), ["b"]);
    }

    #[test]
    fn new_dir_clears_marks() {
        let mut p = loaded("/x", vec![f("a", 1)]);
        p.mark_all(true);
        p.set_listing(PathBuf::from("/y"), vec![f("a", 1)], None);
        assert!(marked_names(&p).is_empty());
    }

    #[test]
    fn mask_and_invert_touch_files_only() {
        let mut p = loaded("/x", vec![d("src.rs"), f("a.rs", 1), f("b.txt", 1)]);
        p.mark_by_mask(&Mask::parse("*.rs"), true);
        assert_eq!(marked_names(&p), ["a.rs"]);
        p.invert();
        assert_eq!(marked_names(&p), ["b.txt"]);
        p.mark_by_mask(&Mask::parse("*"), false);
        assert!(marked_names(&p).is_empty());
    }

    #[test]
    fn mark_all_includes_dirs() {
        let mut p = loaded("/x", vec![d("sub"), f("a", 1)]);
        p.mark_all(true);
        assert_eq!(marked_names(&p), ["sub", "a"]);
        p.mark_all(false);
        assert!(marked_names(&p).is_empty());
    }

    #[test]
    fn marked_totals_count_only_marked() {
        let mut p = loaded("/x", vec![d("sub"), f("a", 1000), f("b", 24)]);
        p.set_cursor(1);
        p.toggle_mark_and_move(1); // sub
        p.toggle_mark(); // a
        assert_eq!(
            p.marked_totals(),
            Totals {
                bytes: 1000,
                files: 1,
                dirs: 1
            }
        );
    }

    #[test]
    fn targets_are_marked_or_cursor_never_parent() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
        assert!(p.targets().is_empty()); // cursor on ".."
        p.set_cursor(1);
        assert_eq!(p.targets(), [PathBuf::from("/x/a")]);
        p.set_cursor(2);
        p.toggle_mark(); // mark b
        p.set_cursor(1); // cursor on a, but marks win
        assert_eq!(p.targets(), [PathBuf::from("/x/b")]);
    }

    #[test]
    fn unmark_removes_given_paths() {
        let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
        p.mark_all(true);
        p.unmark(&[PathBuf::from("/x/a")]);
        assert_eq!(marked_names(&p), ["b"]);
    }

    #[test]
    fn navigation_paths() {
        let mut p = loaded("/x/y", vec![d("sub"), f("file", 1)]);
        // cursor on ".."
        assert_eq!(
            p.enter_path(),
            Some((PathBuf::from("/x"), Some("y".into())))
        );
        assert_eq!(p.parent_path(), Some((PathBuf::from("/x"), "y".into())));
        p.move_cursor(1);
        assert_eq!(p.enter_path(), Some((PathBuf::from("/x/y/sub"), None)));
        p.move_cursor(1);
        assert_eq!(p.enter_path(), None);
        assert_eq!(loaded("/", vec![]).parent_path(), None);
    }
}
