# Фаза 1: core — листинг, сортировка, Panel. План реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In `shagoff-core`, list a directory, sort it the TC way, and hold a panel's state (cwd, entries, cursor, sort). All of it is unit-tested.

**Architecture:** There are three modules, one per responsibility:
- `listing` reads the filesystem;
- `sort` is a pure comparator;
- `panel` holds state and never touches the filesystem.

The UI (phase 2) calls `listing::scan` in the background and passes the result to `Panel::set_listing`.

**Tech Stack:** Rust 2024 and std only. `tempfile` is a dev-dependency.

**Spec:** `.claude/docs/specs/2026-10-02-core-panel.md`

## Global Constraints

- `crates/core` must not depend on libcosmic or any UI crate. No new runtime dependencies, only std.
- Edition 2024. `cargo clippy --all-targets -- -D warnings` must be clean.
- Folders always sort above files, in both directions. `..` is always the first row, except at `/`.
- Name comparison is natural and case-insensitive (`file2 < file10`).
- Branch: `feat/core-panel`. Commit after every task.

## Review Focus

1. **Unreadable or missing directory.** `scan` returns `Err` and the panel is unchanged. Covered by test `scan_missing_dir_is_error` in Task 1. The UI calls `set_listing` only on `Ok`.
2. **Empty directory and the `/` root.** There is no `..` row and the list is empty. `current()` returns `None`, and cursor moves don't panic. Covered by `empty_root_has_no_rows_and_cursor_is_safe` in Task 3.
3. **Rescan after the file under the cursor was deleted.** The cursor stays at the same index, clamped to the end of the list. Covered by `rescan_keeps_index_when_name_gone` and `rescan_clamps_when_list_shrinks` in Task 3.
4. **File name that isn't valid UTF-8.** It is listed through `to_string_lossy`, with no panic and no skip. Covered by `scan_non_utf8_name_is_listed` in Task 1.
5. **Names that differ only by case, or by leading zeros.** The order is still deterministic, so the cursor doesn't jump between rescans. Covered by `natural_cmp_tiebreak_is_deterministic` in Task 2.

---

### Task 1: `listing` — Entry and scan

**Files:**
- Modify: `crates/core/Cargo.toml` (add `[dev-dependencies] tempfile = "3"`)
- Modify: `crates/core/src/lib.rs`
- Create: `crates/core/src/listing.rs` (code and `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub enum Kind { Dir, File }` (derives `Clone, Copy, Debug, PartialEq, Eq`)
  - `pub struct Entry { pub name: String, pub ext: String, pub size: u64, pub mtime: SystemTime, pub kind: Kind, pub is_link: bool }` (derives `Clone, Debug, PartialEq, Eq`)
  - `impl Entry { pub fn is_dir(&self) -> bool }`
  - `pub fn scan(path: &Path, show_hidden: bool) -> io::Result<Vec<Entry>>`

- [ ] **Step 1: Add the dev-dependency and the module**

  In `crates/core/Cargo.toml`, add:
  ```toml
  [dev-dependencies]
  tempfile = "3"
  ```

  Then replace the whole of `crates/core/src/lib.rs` with:
  ```rust
  //! UI-free core of Shagoff Commander: panel state, directory listing, file operations.
  //! Must not depend on libcosmic so everything here stays unit-testable.

  pub mod listing;
  ```

- [ ] **Step 2: Write the failing tests**

  Create `crates/core/src/listing.rs` with a stub and the tests:
  ```rust
  use std::{
      fs, io,
      path::Path,
      time::{SystemTime, UNIX_EPOCH},
  };

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Kind {
      Dir,
      File,
  }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Entry {
      pub name: String,
      /// Extension of files only (`archive.tar.gz` → `gz`, `.bashrc` → empty); always empty for dirs.
      pub ext: String,
      pub size: u64,
      pub mtime: SystemTime,
      pub kind: Kind,
      pub is_link: bool,
  }

  impl Entry {
      pub fn is_dir(&self) -> bool {
          self.kind == Kind::Dir
      }
  }

  pub fn scan(_path: &Path, _show_hidden: bool) -> io::Result<Vec<Entry>> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use std::os::unix::fs::symlink;

      fn find<'a>(v: &'a [Entry], name: &str) -> &'a Entry {
          v.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("{name} not listed"))
      }

      fn fixture() -> tempfile::TempDir {
          let d = tempfile::tempdir().unwrap();
          fs::write(d.path().join("a.txt"), "hello").unwrap();
          fs::write(d.path().join("archive.tar.gz"), "").unwrap();
          fs::write(d.path().join(".bashrc"), "").unwrap();
          fs::write(d.path().join("noext"), "").unwrap();
          fs::create_dir(d.path().join("sub.d")).unwrap();
          symlink(d.path().join("sub.d"), d.path().join("link_dir")).unwrap();
          symlink(d.path().join("nope"), d.path().join("broken")).unwrap();
          d
      }

      #[test]
      fn scan_lists_files_dirs_and_links() {
          let d = fixture();
          let v = scan(d.path(), true).unwrap();
          assert_eq!(v.len(), 7);
          let a = find(&v, "a.txt");
          assert_eq!((a.kind, a.size, a.ext.as_str(), a.is_link), (Kind::File, 5, "txt", false));
          let sub = find(&v, "sub.d");
          assert_eq!((sub.kind, sub.ext.as_str()), (Kind::Dir, ""));
          let link = find(&v, "link_dir");
          assert_eq!((link.kind, link.is_link), (Kind::Dir, true));
          let broken = find(&v, "broken");
          assert_eq!((broken.kind, broken.size, broken.is_link), (Kind::File, 0, true));
      }

      #[test]
      fn scan_ext_rules() {
          let d = fixture();
          let v = scan(d.path(), true).unwrap();
          assert_eq!(find(&v, "archive.tar.gz").ext, "gz");
          assert_eq!(find(&v, ".bashrc").ext, "");
          assert_eq!(find(&v, "noext").ext, "");
      }

      #[test]
      fn scan_hides_dotfiles_unless_asked() {
          let d = fixture();
          let v = scan(d.path(), false).unwrap();
          assert!(v.iter().all(|e| !e.name.starts_with('.')));
          assert_eq!(v.len(), 6);
      }

      #[test]
      fn scan_missing_dir_is_error() {
          let d = tempfile::tempdir().unwrap();
          assert!(scan(&d.path().join("missing"), true).is_err());
      }

      #[test]
      fn scan_non_utf8_name_is_listed() {
          use std::ffi::OsStr;
          use std::os::unix::ffi::OsStrExt;
          let d = tempfile::tempdir().unwrap();
          fs::write(d.path().join(OsStr::from_bytes(b"bad\xffname")), "").unwrap();
          let v = scan(d.path(), true).unwrap();
          assert_eq!(v.len(), 1);
          assert!(v[0].name.starts_with("bad"));
      }
  }
  ```

- [ ] **Step 3: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core listing`

  Expected: the tests panic with `not yet implemented`, except `scan_missing_dir_is_error`, which also panics.

- [ ] **Step 4: Implement `scan` and `ext_of`**

  Replace the stub `scan` with:
  ```rust
  /// Lists `path` without `..`. Unreadable items are skipped; only failing to read the dir itself is an error.
  pub fn scan(path: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
      let mut out = Vec::new();
      for item in fs::read_dir(path)? {
          let Ok(item) = item else { continue };
          let name = item.file_name().to_string_lossy().into_owned();
          if !show_hidden && name.starts_with('.') {
              continue;
          }
          // DirEntry::metadata does not follow symlinks.
          let Ok(lmeta) = item.metadata() else { continue };
          let is_link = lmeta.file_type().is_symlink();
          let target = if is_link { fs::metadata(item.path()).ok() } else { Some(lmeta.clone()) };
          let (kind, size, meta) = match &target {
              Some(m) if m.is_dir() => (Kind::Dir, 0, m),
              Some(m) => (Kind::File, m.len(), m),
              None => (Kind::File, 0, &lmeta), // broken symlink
          };
          let ext = if kind == Kind::File { ext_of(&name) } else { String::new() };
          out.push(Entry {
              mtime: meta.modified().unwrap_or(UNIX_EPOCH),
              name,
              ext,
              size,
              kind,
              is_link,
          });
      }
      Ok(out)
  }

  fn ext_of(name: &str) -> String {
      match name.rfind('.') {
          Some(i) if i > 0 => name[i + 1..].to_string(),
          _ => String::new(),
      }
  }
  ```

- [ ] **Step 5: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core listing`

  Expected: 5 passed.

- [ ] **Step 6: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core Cargo.lock
  git commit -m "core: listing::scan with Entry (dirs, files, symlinks, ext)"
  ```

---

### Task 2: `sort` — natural comparison and the TC ordering

**Files:**
- Modify: `crates/core/src/lib.rs` (add `pub mod sort;`)
- Create: `crates/core/src/sort.rs`

**Interfaces:**
- Consumes: `listing::{Entry, Kind}`, `Entry::is_dir()` from Task 1.
- Produces:
  - `pub enum SortKey { Name, Ext, Size, Date }` (derives `Clone, Copy, Debug, Default, PartialEq, Eq`, with `Name` as the default)
  - `pub struct Sort { pub key: SortKey, pub asc: bool }` (derives `Clone, Copy, Debug, PartialEq, Eq`; `Default` gives `Name` ascending)
  - `pub fn natural_cmp(a: &str, b: &str) -> Ordering`
  - `pub fn sort_entries(entries: &mut [Entry], sort: Sort)`

- [ ] **Step 1: Write the failing tests**

  Add `pub mod sort;` to `lib.rs`. Then create `crates/core/src/sort.rs`:
  ```rust
  use crate::listing::Entry;
  use std::cmp::Ordering;

  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
  pub enum SortKey {
      #[default]
      Name,
      Ext,
      Size,
      Date,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct Sort {
      pub key: SortKey,
      pub asc: bool,
  }

  impl Default for Sort {
      fn default() -> Self {
          Self { key: SortKey::Name, asc: true }
      }
  }

  pub fn natural_cmp(_a: &str, _b: &str) -> Ordering {
      todo!()
  }

  pub fn sort_entries(_entries: &mut [Entry], _sort: Sort) {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::listing::Kind;
      use std::time::{Duration, UNIX_EPOCH};

      fn e(name: &str, kind: Kind, size: u64, secs: u64) -> Entry {
          let ext = match (kind, name.rfind('.')) {
              (Kind::File, Some(i)) if i > 0 => name[i + 1..].to_string(),
              _ => String::new(),
          };
          Entry {
              name: name.into(),
              ext,
              size,
              mtime: UNIX_EPOCH + Duration::from_secs(secs),
              kind,
              is_link: false,
          }
      }

      fn names(v: &[Entry]) -> Vec<&str> {
          v.iter().map(|e| e.name.as_str()).collect()
      }

      #[test]
      fn natural_cmp_numbers_and_case() {
          use Ordering::*;
          assert_eq!(natural_cmp("file2", "file10"), Less);
          assert_eq!(natural_cmp("File10", "file2"), Greater);
          assert_eq!(natural_cmp("abc", "ABD"), Less);
          assert_eq!(natural_cmp("a", "ab"), Less);
          assert_eq!(natural_cmp("", "a"), Less);
          assert_eq!(natural_cmp("x", "x"), Equal);
          assert_eq!(natural_cmp("v1.9", "v1.10"), Less);
      }

      #[test]
      fn natural_cmp_tiebreak_is_deterministic() {
          use Ordering::*;
          // equal ignoring case / leading zeros → fall back to plain byte order, never Equal
          assert_eq!(natural_cmp("A", "a"), Less);
          assert_eq!(natural_cmp("a", "A"), Greater);
          assert_eq!(natural_cmp("file01", "file1"), Less);
          assert_eq!(natural_cmp("file1", "file01"), Greater);
      }

      #[test]
      fn dirs_first_then_name() {
          let mut v = vec![
              e("b.txt", Kind::File, 1, 0),
              e("Zdir", Kind::Dir, 0, 0),
              e("a10", Kind::File, 1, 0),
              e("adir", Kind::Dir, 0, 0),
              e("a2", Kind::File, 1, 0),
          ];
          sort_entries(&mut v, Sort::default());
          assert_eq!(names(&v), ["adir", "Zdir", "a2", "a10", "b.txt"]);
      }

      #[test]
      fn descending_keeps_dirs_on_top() {
          let mut v = vec![
              e("a", Kind::File, 1, 0),
              e("d1", Kind::Dir, 0, 0),
              e("b", Kind::File, 1, 0),
              e("d2", Kind::Dir, 0, 0),
          ];
          sort_entries(&mut v, Sort { key: SortKey::Name, asc: false });
          assert_eq!(names(&v), ["d2", "d1", "b", "a"]);
      }

      #[test]
      fn by_size_dirs_by_name_files_by_size_then_name() {
          let mut v = vec![
              e("big", Kind::File, 100, 0),
              e("zdir", Kind::Dir, 0, 0),
              e("small2", Kind::File, 1, 0),
              e("adir", Kind::Dir, 0, 0),
              e("small1", Kind::File, 1, 0),
          ];
          sort_entries(&mut v, Sort { key: SortKey::Size, asc: true });
          assert_eq!(names(&v), ["adir", "zdir", "small1", "small2", "big"]);
      }

      #[test]
      fn by_ext_and_by_date() {
          let mut v = vec![
              e("x.rs", Kind::File, 0, 30),
              e("y.md", Kind::File, 0, 10),
              e("a.rs", Kind::File, 0, 20),
          ];
          sort_entries(&mut v, Sort { key: SortKey::Ext, asc: true });
          assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
          sort_entries(&mut v, Sort { key: SortKey::Date, asc: true });
          assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
          sort_entries(&mut v, Sort { key: SortKey::Date, asc: false });
          assert_eq!(names(&v), ["x.rs", "a.rs", "y.md"]);
      }
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core sort`

  Expected: 6 tests panic with `not yet implemented`.

- [ ] **Step 3: Implement**

  Replace the two stubs with:
  ```rust
  /// Case-insensitive comparison where digit runs compare as numbers (`file2 < file10`).
  /// Names equal under that rule fall back to byte order, so the result is never `Equal` for different strings.
  pub fn natural_cmp(a: &str, b: &str) -> Ordering {
      let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
      loop {
          let ord = match (x.peek().copied(), y.peek().copied()) {
              (None, None) => return a.cmp(b),
              (None, Some(_)) => return Ordering::Less,
              (Some(_), None) => return Ordering::Greater,
              (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                  let (n, m) = (take_digits(&mut x), take_digits(&mut y));
                  let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                  n.len().cmp(&m.len()).then_with(|| n.cmp(m))
              }
              (Some(c), Some(d)) => {
                  x.next();
                  y.next();
                  c.to_lowercase().cmp(d.to_lowercase())
              }
          };
          if ord != Ordering::Equal {
              return ord;
          }
      }
  }

  fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
      let mut s = String::new();
      while let Some(c) = it.next_if(char::is_ascii_digit) {
          s.push(c);
      }
      s
  }

  /// TC order: dirs above files in both directions; dirs sort by name when sorting by size.
  pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
      entries.sort_by(|a, b| {
          match (a.is_dir(), b.is_dir()) {
              (true, false) => return Ordering::Less,
              (false, true) => return Ordering::Greater,
              _ => {}
          }
          let key = if a.is_dir() && sort.key == SortKey::Size { SortKey::Name } else { sort.key };
          let ord = match key {
              SortKey::Name => Ordering::Equal,
              SortKey::Ext => natural_cmp(&a.ext, &b.ext),
              SortKey::Size => a.size.cmp(&b.size),
              SortKey::Date => a.mtime.cmp(&b.mtime),
          }
          .then_with(|| natural_cmp(&a.name, &b.name));
          if sort.asc { ord } else { ord.reverse() }
      });
  }
  ```

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core sort`

  Expected: 6 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core
  git commit -m "core: natural name comparison and TC sort order"
  ```

---

### Task 3: `panel` — Panel state

**Files:**
- Modify: `crates/core/src/lib.rs` (add `pub mod panel;`)
- Create: `crates/core/src/panel.rs`

**Interfaces:**
- Consumes: `listing::{Entry, Kind}` from Task 1; `sort::{Sort, SortKey, sort_entries}` from Task 2.
- Produces (used by the UI in phase 2):
  - `pub const PARENT: &str = ".."`
  - `pub struct Panel` with:
    - `new(cwd: PathBuf) -> Self`
    - getters `cwd() -> &Path`, `entries() -> &[Entry]`, `cursor() -> usize`, `sort() -> Sort`, `show_hidden() -> bool`
    - `set_show_hidden(bool)`
    - `set_listing(cwd: PathBuf, entries: Vec<Entry>, focus: Option<&str>)`
    - `move_cursor(isize)`, `cursor_home()`, `cursor_end()`
    - `set_sort(SortKey)`
    - `current() -> Option<&Entry>`
    - `parent_path() -> Option<(PathBuf, String)>`
    - `enter_path() -> Option<(PathBuf, Option<String>)>`

- [ ] **Step 1: Write the failing tests**

  Add `pub mod panel;` to `lib.rs`. Then create `crates/core/src/panel.rs`:
  ```rust
  use crate::listing::{Entry, Kind};
  use crate::sort::{Sort, SortKey, sort_entries};
  use std::path::{Path, PathBuf};
  use std::time::UNIX_EPOCH;

  /// Name of the synthetic "go up" row.
  pub const PARENT: &str = "..";

  /// One panel's state. Never touches the filesystem: the UI scans and hands results to `set_listing`.
  #[derive(Debug)]
  pub struct Panel {
      cwd: PathBuf,
      entries: Vec<Entry>,
      cursor: usize,
      sort: Sort,
      show_hidden: bool,
  }

  impl Panel {
      pub fn new(cwd: PathBuf) -> Self {
          Self { cwd, entries: Vec::new(), cursor: 0, sort: Sort::default(), show_hidden: false }
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
      pub fn set_show_hidden(&mut self, on: bool) {
          self.show_hidden = on;
      }
      pub fn set_listing(&mut self, _cwd: PathBuf, _entries: Vec<Entry>, _focus: Option<&str>) {
          todo!()
      }
      pub fn move_cursor(&mut self, _delta: isize) {
          todo!()
      }
      pub fn cursor_home(&mut self) {
          todo!()
      }
      pub fn cursor_end(&mut self) {
          todo!()
      }
      pub fn set_sort(&mut self, _key: SortKey) {
          todo!()
      }
      pub fn current(&self) -> Option<&Entry> {
          todo!()
      }
      pub fn parent_path(&self) -> Option<(PathBuf, String)> {
          todo!()
      }
      pub fn enter_path(&self) -> Option<(PathBuf, Option<String>)> {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use std::time::Duration;

      fn f(name: &str, size: u64) -> Entry {
          Entry { name: name.into(), ext: String::new(), size, mtime: UNIX_EPOCH, kind: Kind::File, is_link: false }
      }
      fn d(name: &str) -> Entry {
          Entry { kind: Kind::Dir, ..f(name, 0) }
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
          p.set_listing(PathBuf::from("/x/y"), vec![f("q", 1), f("r", 1), f("s", 1)], None);
          assert_eq!(p.cursor(), 0);
          assert_eq!(p.cwd(), Path::new("/x/y"));
      }

      #[test]
      fn rescan_keeps_cursor_on_same_name() {
          let mut p = loaded("/x", vec![f("b", 1), f("c", 1)]);
          p.cursor_end(); // on "c"
          p.set_listing(PathBuf::from("/x"), vec![f("a", 1), f("b", 1), f("c", 1)], None);
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
          assert_eq!(p.sort(), Sort { key: SortKey::Size, asc: true });
          assert_eq!(names(&p), ["..", "b", "c", "a"]);
          assert_eq!(p.current().unwrap().name, "a");
          p.set_sort(SortKey::Size);
          assert_eq!(p.sort(), Sort { key: SortKey::Size, asc: false });
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
      fn navigation_paths() {
          let mut p = loaded("/x/y", vec![d("sub"), f("file", 1)]);
          // cursor on ".."
          assert_eq!(p.enter_path(), Some((PathBuf::from("/x"), Some("y".into()))));
          assert_eq!(p.parent_path(), Some((PathBuf::from("/x"), "y".into())));
          p.move_cursor(1);
          assert_eq!(p.enter_path(), Some((PathBuf::from("/x/y/sub"), None)));
          p.move_cursor(1);
          assert_eq!(p.enter_path(), None);
          assert_eq!(loaded("/", vec![]).parent_path(), None);
      }
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core panel`

  Expected: 11 tests panic with `not yet implemented`.

- [ ] **Step 3: Implement**

  Replace the `todo!()` methods with the code below. Also add the private helpers `index_of` and `parent_row` to the same `impl` block, plus the free function `parent_entry` (all three are in the code).
  ```rust
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

      pub fn current(&self) -> Option<&Entry> {
          self.entries.get(self.cursor)
      }

      /// Parent dir and the name to focus there (the dir we leave).
      pub fn parent_path(&self) -> Option<(PathBuf, String)> {
          let parent = self.cwd.parent()?;
          let name = self.cwd.file_name()?.to_string_lossy().into_owned();
          Some((parent.to_path_buf(), name))
      }

      /// Where Enter leads: `..` → parent (with focus), a dir → inside it, a file → `None`.
      pub fn enter_path(&self) -> Option<(PathBuf, Option<String>)> {
          let e = self.current()?;
          if e.name == PARENT {
              return self.parent_path().map(|(p, n)| (p, Some(n)));
          }
          e.is_dir().then(|| (self.cwd.join(&e.name), None))
      }

      fn index_of(&self, name: &str) -> Option<usize> {
          self.entries.iter().position(|e| e.name == name)
      }

      fn parent_row(&self) -> bool {
          self.entries.first().is_some_and(|e| e.name == PARENT)
      }
  ```

  After the `impl` block, add:
  ```rust
  fn parent_entry() -> Entry {
      Entry {
          name: PARENT.into(),
          ext: String::new(),
          size: 0,
          mtime: UNIX_EPOCH,
          kind: Kind::Dir,
          is_link: false,
      }
  }
  ```

  Once both are in, the test helper `f` can use `UNIX_EPOCH` through `use super::*`.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core panel`

  Expected: 11 passed.

- [ ] **Step 5: Run the full verification and commit**
  ```bash
  just verify
  git add crates/core
  git commit -m "core: Panel state with TC cursor rules and sorting"
  ```
  Expected: `just verify` reports 22 tests passed in total and no warnings.

---

### Task 4: Docs and PR

**Files:**
- Modify: `.claude/docs/ROADMAP.md` (tick phases 0 and 1)
- Modify: `CLAUDE.md` (replace the sentence "Only bootstrap exists so far (an empty window)." with "Implemented so far: `crates/core` (`listing`, `sort`, `panel`); the app is still an empty window.")

- [ ] **Step 1: Edit both files as described above**

- [ ] **Step 2: Commit, push and open the PR**
  ```bash
  git add .claude CLAUDE.md
  git commit -m "docs: phase 1 spec, plan, roadmap"
  git push -u origin feat/core-panel
  gh pr create --title "core: listing, sort, Panel (phase 1)" --body "Spec: .claude/docs/specs/2026-10-02-core-panel.md
  Plan: .claude/docs/plans/01-core-panel.md"
  gh pr checks --watch
  ```

- [ ] **Step 3: Merge only after the user approves:** `gh pr merge --merge --delete-branch`
