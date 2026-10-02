# Quick Search and Panel Filter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Alt+letter quick search (jumps the cursor to the first name with that prefix) and a Ctrl+S filter that hides non-matching entries, as in TC.

**Architecture:**
- Matching (`quicksearch::matches`) lives in core, and so does the filtered view (`Panel` keeps the full list plus a visible one).
- The app adds `App.search: Option<Search>`, rendered as a focused `text_input` in place of the pane's status line.
- Keys pass through one gate while the field is open: ↑/↓ act inside the search, any other key closes the field and acts as usual.

**Tech Stack:** Rust 2024, libcosmic rev `ef490df50b0a05a21c494c3f75737581bf0b39d9`.

**Spec:** `.claude/docs/specs/07-quick-search.md`

## Global Constraints

- `crates/core` must not depend on libcosmic. Matching and filtering logic goes in core, with tests.
- Every new UI string goes in both `en` and `ru` ftl.
- Letter keys match the logical character for search (so Alt+ф searches «ф»). Ctrl+S matches the physical `KeyS`. Modifiers must match exactly.
- `..` never matches a search and always stays visible.
- Never send keys to the desktop with `wtype`.
- `just verify` must pass before each task's commit.

## Review Focus

1. **Marks on entries the filter hides.** F8 must never act on invisible files. Pinned by `panel::tests::filter_drops_marks_of_hidden_entries` (Task 2).
2. **A rescan while filtered.** The watcher or Ctrl+R must keep the filter and show new matching files; entering another dir must drop it. Pinned by `filter_survives_rescan_and_drops_on_new_dir` (Task 2).
3. **Search with no match anywhere.** The letter is rejected and nothing else changes. With Alt+letter as the very first key, no field opens. Pinned by `alt_letter_without_match_opens_nothing` and `search_rejects_letter_without_match` (Task 4).
4. **Escape layering.** With the field open, Escape closes it; in filter mode it also drops the filter. With the field closed and a filter set, Escape drops the filter. During a running job, Escape still cancels the job. Pinned by `escape_closes_search_then_clears_filter` (Task 4).
5. **Uppercase, Cyrillic and `?` in the pattern.** `Док` must find `документы`. Pinned by `quicksearch::tests::case_and_cyrillic` (Task 1).

---

### Task 1: `quicksearch::matches`

**Files:**
- Create: `crates/core/src/quicksearch.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod quicksearch;`), `crates/core/src/mask.rs` (`fn glob` → `pub(crate) fn glob`)

**Interfaces:**
- Produces: `quicksearch::matches(pattern: &str, name: &str) -> bool`

- [ ] **Step 1: Failing tests** (new file, with `matches` stubbed as `todo!()`)

```rust
//! TC quick search / filter matching: name prefix, `*` = anywhere, case-insensitive.

use crate::mask::glob;
use crate::panel::PARENT;

pub fn matches(_pattern: &str, _name: &str) -> bool {
    todo!()
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
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p shagoff-core quicksearch`
Expected: FAIL (`not yet implemented`).

- [ ] **Step 3: Implement**

```rust
/// `doc` → glob `doc*` (prefix); `*doc` → `*doc*` (anywhere): appending `*` covers both.
pub fn matches(pattern: &str, name: &str) -> bool {
    name != PARENT && glob(&format!("{}*", pattern.to_lowercase()), &name.to_lowercase())
}
```

In `mask.rs`, change `fn glob(` to `pub(crate) fn glob(`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-core quicksearch && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/core && git commit -m "core: quick search matching"
```

---

### Task 2: `Panel` — find and filter

**Files:**
- Modify: `crates/core/src/panel.rs`

**Interfaces:**
- Consumes: `quicksearch::matches` (Task 1)
- Produces:
  - `Panel::find(&self, pattern: &str, from: usize, forward: bool) -> Option<usize>`. Checks `from` first, then walks forward (`from+1, …`) or backward (`from−1, …`), wrapping. `from` is taken modulo `entries.len()`. Returns `None` if the list is empty or nothing matches.
  - `Panel::filter(&self) -> Option<&str>`
  - `Panel::set_filter(&mut self, pattern: Option<String>)`

- [ ] **Step 1: Failing tests** (add to `panel.rs` tests; add stubs `find`/`filter`/`set_filter` with `todo!()`)

```rust
    fn three() -> Panel {
        loaded("/x", vec![d("docs"), f("data.txt", 1), f("readme", 2), f("dump", 3)])
        // sorted: .., docs, data.txt, dump, readme
    }

    #[test]
    fn find_first_from_top_skips_parent() {
        let p = three();
        assert_eq!(p.find("d", 0, true), Some(1)); // docs
        assert_eq!(p.find("zzz", 0, true), None);
    }

    #[test]
    fn find_next_and_prev_wrap() {
        let p = three();
        assert_eq!(p.find("d", 2, true), Some(2)); // from is inclusive
        assert_eq!(p.find("d", 4, true), Some(1)); // wraps past readme to docs
        assert_eq!(p.find("d", 0, false), Some(3)); // backward from .. wraps to dump
        assert_eq!(p.find("d", 7, true), Some(2)); // from modulo len (7 % 5 = 2)
    }

    #[test]
    fn filter_shows_parent_and_matches_only() {
        let mut p = three();
        p.set_filter(Some("d".into()));
        assert_eq!(names(&p), ["..", "docs", "data.txt", "dump"]);
        assert_eq!(p.filter(), Some("d"));
        p.set_filter(None);
        assert_eq!(names(&p), ["..", "docs", "data.txt", "dump", "readme"]);
        assert_eq!(p.filter(), None);
    }

    #[test]
    fn filter_keeps_cursor_on_same_name_or_clamps() {
        let mut p = three();
        p.set_cursor(3); // dump
        p.set_filter(Some("du".into()));
        assert_eq!(p.current().unwrap().name, "dump");
        p.set_cursor(1);
        p.set_filter(Some("zzz".into()));
        assert_eq!(names(&p), [".."]);
        assert_eq!(p.cursor(), 0);
    }

    #[test]
    fn filter_drops_marks_of_hidden_entries() {
        let mut p = three();
        p.mark_all(true);
        p.set_filter(Some("da".into()));
        p.set_filter(None);
        let marked: Vec<_> = p.entries().iter().filter(|e| p.is_marked(e)).map(|e| e.name.as_str()).collect();
        assert_eq!(marked, ["data.txt"]);
    }

    #[test]
    fn filter_survives_rescan_and_drops_on_new_dir() {
        let mut p = three();
        p.set_filter(Some("d".into()));
        p.set_listing("/x".into(), vec![d("docs"), f("dart", 1), f("zeta", 1)], None);
        assert_eq!(names(&p), ["..", "docs", "dart"]);
        assert_eq!(p.filter(), Some("d"));
        p.set_listing("/y".into(), vec![f("a", 1), f("b", 1)], None);
        assert_eq!(names(&p), ["..", "a", "b"]);
        assert_eq!(p.filter(), None);
    }

    #[test]
    fn sort_with_filter_keeps_both_lists_sorted() {
        let mut p = three();
        p.set_filter(Some("d".into()));
        p.set_sort(SortKey::Size); // data.txt 1, dump 3 (dirs first)
        p.set_sort(SortKey::Size); // descending
        assert_eq!(names(&p), ["..", "docs", "dump", "data.txt"]);
        p.set_filter(None);
        assert_eq!(names(&p), ["..", "docs", "readme", "dump", "data.txt"]);
    }
```
If `three()`'s natural sort order differs from the comment, fix the expected indices to match the real `names(&p)` printed by a failing assertion before the implementation (the order comes from existing `sort_entries`, not from this task).

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p shagoff-core panel`
Expected: the new tests panic on `todo!()`; old tests pass.

- [ ] **Step 3: Implement**

Add fields:
```rust
    /// Full sorted listing without `..`; `entries` is `..` + the part of it the filter lets through.
    all: Vec<Entry>,
    /// Quick filter (Ctrl+S); `None` = show everything.
    filter: Option<String>,
```
(initialise as `Vec::new()` / `None` in `new`).

Replace the tail of `set_listing` (from `sort_entries(&mut entries, self.sort);` to the end) with:
```rust
        sort_entries(&mut entries, self.sort);
        if !same_dir {
            self.marked.clear();
            self.filter = None;
        }
        self.cwd = cwd;
        self.all = entries;
        self.rebuild(keep, old);
```
Add:
```rust
    /// Recompute the visible rows from `all` and the filter; drop marks that are no longer visible;
    /// put the cursor on `keep` if visible, else on `old`, clamped.
    fn rebuild(&mut self, keep: Option<String>, old: usize) {
        let mut entries: Vec<Entry> = match &self.filter {
            Some(f) => self.all.iter().filter(|e| quicksearch::matches(f, &e.name)).cloned().collect(),
            None => self.all.clone(),
        };
        if self.cwd.parent().is_some() {
            entries.insert(0, parent_entry());
        }
        let visible: HashSet<&OsString> = entries.iter().map(|e| &e.os_name).collect();
        self.marked.retain(|k| visible.contains(k));
        self.entries = entries;
        self.cursor = keep.and_then(|n| self.index_of(&n)).unwrap_or(old);
        self.move_cursor(0); // clamp
    }

    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub fn set_filter(&mut self, pattern: Option<String>) {
        self.filter = pattern.filter(|p| !p.is_empty());
        let keep = self.current().map(|e| e.name.clone());
        self.rebuild(keep, 0);
    }

    pub fn find(&self, pattern: &str, from: usize, forward: bool) -> Option<usize> {
        let n = self.entries.len();
        (0..n)
            .map(|k| if forward { (from + k) % n } else { (from % n + n - k) % n })
            .find(|&i| quicksearch::matches(pattern, &self.entries[i].name))
    }
```
`find` with `n == 0`: the range is empty, so it returns `None` before any `% 0`.

Note the behaviour change: the old `set_listing` kept marks on any name still present after a rescan, and `rebuild` keeps marks on visible names. Without a filter these are the same, so existing tests should still pass.

`set_sort`: sort `all` as well and rebuild, keeping the name:
```rust
    pub fn set_sort(&mut self, key: SortKey) {
        let asc = self.sort.key != key || !self.sort.asc;
        self.sort = Sort { key, asc };
        let name = self.current().map(|e| e.name.clone());
        sort_entries(&mut self.all, self.sort);
        let old = self.cursor;
        self.rebuild(name, old);
    }
```
Import: `use crate::quicksearch;`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-core && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/core && git commit -m "core: panel find and quick filter"
```

---

### Task 3: keymap — `QuickSearch(char)`, `QuickFilter`

**Files:**
- Modify: `crates/app/src/keymap.rs`, `crates/app/src/app.rs` (a placeholder arm only)

**Interfaces:**
- Produces: `Action::QuickSearch(char)`, `Action::QuickFilter`

- [ ] **Step 1: Failing tests** (keymap tests)

```rust
    #[test]
    fn alt_letter_is_quick_search() {
        assert_eq!(chr("d", Code::KeyD, ALT), Some(Action::QuickSearch('d')));
        assert_eq!(chr("в", Code::KeyD, ALT), Some(Action::QuickSearch('в')));
        assert_eq!(chr("1", Code::Digit1, ALT), Some(Action::QuickSearch('1')));
        assert_eq!(chr("D", Code::KeyD, ALT | Modifiers::SHIFT), None);
        assert_eq!(chr("d", Code::KeyD, ALT | CTRL), None);
        assert_eq!(named(Named::F1, ALT), Some(Action::Drives(0))); // still drives
    }

    #[test]
    fn ctrl_s_is_quick_filter() {
        assert_eq!(chr("s", Code::KeyS, CTRL), Some(Action::QuickFilter));
        assert_eq!(chr("ы", Code::KeyS, CTRL), Some(Action::QuickFilter));
    }
```
In the existing `alt_only_f1_f2` test, change `assert_eq!(chr("a", Code::KeyA, ALT), None);` to `assert_eq!(chr("a", Code::KeyA, ALT), Some(Action::QuickSearch('a')));`. The test's purpose (only F1/F2 among named keys) is unchanged.

- [ ] **Step 2: Run — expect FAIL** (no such variants)

Run: `cargo test -p shagoff-commander keymap`

- [ ] **Step 3: Implement**

Add variants `QuickSearch(char)` and `QuickFilter` to `Action`, with doc comments.

Alt block:
```rust
    if mods.alt() {
        // Alt+F1/F2: drives; Alt+symbol: quick search (TC). Other Alt combos belong to the compositor.
        return match key {
            Key::Named(Named::F1) if mods == Modifiers::ALT => Some(Action::Drives(0)),
            Key::Named(Named::F2) if mods == Modifiers::ALT => Some(Action::Drives(1)),
            Key::Character(s) if mods == Modifiers::ALT => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => Some(Action::QuickSearch(c)),
                    _ => None,
                }
            }
            _ => None,
        };
    }
```
Physical table: `(Physical::Code(Code::KeyS), true, false) => Some(Action::QuickFilter),`

`app.rs` `act()`: add a temporary arm `Action::QuickSearch(_) | Action::QuickFilter => {}`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "keymap: Alt+symbol quick search, Ctrl+S filter"
```

---

### Task 4: app — search field, key routing, Escape, view

**Files:**
- Modify: `crates/app/src/app.rs`, `crates/app/src/view.rs`, both ftl files

**Interfaces:**
- Consumes: `Panel::{find, filter, set_filter}` (Task 2); `Action::{QuickSearch, QuickFilter}` (Task 3)
- Produces:
  - `pub struct Search { pub side: usize, pub text: String, pub filter: bool }`;
  - `App.search: Option<Search>` (pub);
  - `App.input_id` becomes `pub(crate)`;
  - `Message::SearchInput(String)`, `Message::SearchSubmit`.

- [ ] **Step 1: Failing app tests** (in `app.rs` tests; helper `files_app` lists names in pane 0)

```rust
    /// Pane 0 shows `names` (files) in temp_dir.
    fn files_app(names: &[&str]) -> App {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let entries = names.iter().map(|n| entry(n)).collect();
        let _ = app.update(Message::Listed {
            side: 0,
            tab: id,
            generation,
            path: std::env::temp_dir(),
            focus: None,
            result: Ok(entries),
            space: None,
        });
        app
    }

    fn cursor_name(app: &App) -> String {
        app.panes[0].active().panel.current().unwrap().name.clone()
    }

    #[test]
    fn alt_letter_jumps_to_first_match() {
        let mut app = files_app(&["alpha", "beta", "bravo"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        assert_eq!(cursor_name(&app), "beta");
        assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("b"));
        let _ = app.update(Message::SearchInput("br".into()));
        assert_eq!(cursor_name(&app), "bravo");
    }

    #[test]
    fn alt_letter_without_match_opens_nothing() {
        let mut app = files_app(&["alpha"]);
        let _ = app.update(Message::Key(Action::QuickSearch('z')));
        assert!(app.search.is_none());
    }

    #[test]
    fn search_rejects_letter_without_match() {
        let mut app = files_app(&["alpha", "beta"]);
        let _ = app.update(Message::Key(Action::QuickSearch('a')));
        let _ = app.update(Message::SearchInput("az".into()));
        assert_eq!(app.search.as_ref().unwrap().text, "a");
        assert_eq!(cursor_name(&app), "alpha");
    }

    #[test]
    fn down_in_search_goes_to_next_match() {
        let mut app = files_app(&["b1", "x", "b2"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(cursor_name(&app), "b2");
        let _ = app.update(Message::Key(Action::Down)); // wraps
        assert_eq!(cursor_name(&app), "b1");
        assert!(app.search.is_some());
    }

    #[test]
    fn other_key_closes_search_and_acts() {
        let mut app = files_app(&["alpha", "beta"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        let _ = app.update(Message::Key(Action::Copy));
        assert!(app.search.is_none());
        assert!(matches!(app.dialog, Some(Dialog::Input { op: InputOp::Copy, .. })));
    }

    #[test]
    fn enter_in_search_opens_the_dir() {
        let mut app = files_app(&[]);
        // Put a dir in the listing: reuse Listed with a Dir entry.
        let t = app.panes[0].active();
        let (id, cwd) = (t.id, t.panel.cwd().to_path_buf());
        let _ = app.load(0, cwd.clone(), None);
        let generation = app.panes[0].active().pending.as_ref().unwrap().0;
        let mut sub = entry("sub");
        sub.kind = shagoff_core::listing::Kind::Dir;
        let _ = app.update(Message::Listed {
            side: 0, tab: id, generation, path: cwd.clone(), focus: None, result: Ok(vec![sub]), space: None,
        });
        let _ = app.update(Message::Key(Action::QuickSearch('s')));
        let _ = app.update(Message::SearchSubmit);
        assert!(app.search.is_none());
        let pending = app.panes[0].active().pending.as_ref().map(|(_, p)| p.clone());
        assert_eq!(pending, Some(cwd.join("sub")));
    }

    #[test]
    fn ctrl_s_filters_enter_keeps_escape_clears() {
        let mut app = files_app(&["a.rs", "b.md", "c.rs"]);
        let _ = app.update(Message::Key(Action::QuickFilter));
        let _ = app.update(Message::SearchInput("*.rs".into()));
        let shown = |app: &App| app.panes[0].active().panel.entries().len();
        assert_eq!(shown(&app), 3); // .., a.rs, c.rs
        let _ = app.update(Message::SearchSubmit);
        assert!(app.search.is_none());
        assert_eq!(app.panes[0].active().panel.filter(), Some("*.rs"));
        let _ = app.update(Message::DialogCancel); // Escape
        assert_eq!(app.panes[0].active().panel.filter(), None);
        assert_eq!(shown(&app), 4);
    }

    #[test]
    fn escape_closes_search_then_clears_filter() {
        let mut app = files_app(&["a.rs", "b.md"]);
        let _ = app.update(Message::Key(Action::QuickFilter));
        let _ = app.update(Message::SearchInput("a".into()));
        let _ = app.update(Message::DialogCancel); // in filter field: close + clear
        assert!(app.search.is_none());
        assert_eq!(app.panes[0].active().panel.filter(), None);

        let _ = app.update(Message::Key(Action::QuickSearch('a')));
        let _ = app.update(Message::DialogCancel); // in search field: only close
        assert!(app.search.is_none());
        assert_eq!(cursor_name(&app), "a.rs");
    }
```
Note: `files_app` gets a fresh temp_dir panel whose cwd has a parent, so `..` is row 0 (count it in the `shown` numbers).

- [ ] **Step 2: Run — expect FAIL** (compile: no `search`, `SearchInput`, …)

Run: `cargo test -p shagoff-commander`

- [ ] **Step 3: Implement**

`app.rs`:
```rust
/// Quick search (Alt+letter) or filter (Ctrl+S) field, shown instead of the pane's status line.
pub struct Search {
    pub side: usize,
    pub text: String,
    pub filter: bool,
}
```
- `App.search: Option<Search>`, `None` in `build`. Make `input_id: widget::Id` `pub(crate)`.
- Messages `SearchInput(String)` and `SearchSubmit` (doc: typed text / Enter in the field).
- In `act()`, right after the error clear and before `dialog_for`:
```rust
        match action {
            Action::QuickSearch(c) => return self.quick_search(side, c),
            Action::QuickFilter => return self.quick_filter(side),
            _ => {}
        }
```
  and change the temporary arm to `Action::QuickSearch(_) | Action::QuickFilter => {} // handled above`.
- Methods:
```rust
    /// Alt+letter: append to an open search (or start one) if the text still matches something.
    fn quick_search(&mut self, side: usize, c: char) -> Task<Message> {
        let mut text = match &self.search {
            Some(s) if s.side == side && !s.filter => s.text.clone(),
            _ => String::new(),
        };
        text.push(c);
        let Some(i) = self.panes[side].active().panel.find(&text, 0, true) else {
            return Task::none(); // TC: a letter that finds nothing is not taken
        };
        self.search = Some(Search { side, text, filter: false });
        self.active = side;
        let t = self.panes[side].active_mut();
        t.panel.set_cursor(i);
        let tab = t.id;
        Task::batch([widget::text_input::focus(self.input_id.clone()), self.reveal(side, tab)])
    }

    fn quick_filter(&mut self, side: usize) -> Task<Message> {
        let text = self.panes[side].active().panel.filter().unwrap_or_default().to_string();
        self.search = Some(Search { side, text, filter: true });
        self.active = side;
        widget::text_input::focus(self.input_id.clone())
    }
```
- `handle`:
```rust
            Message::SearchInput(text) => {
                let Some(s) = &self.search else { return Task::none() };
                let (side, filter) = (s.side, s.filter);
                let t = self.panes[side].active_mut();
                if filter {
                    t.panel.set_filter(Some(text.clone()));
                } else if !text.is_empty() {
                    match t.panel.find(&text, 0, true) {
                        Some(i) => t.panel.set_cursor(i),
                        None => return Task::none(), // rejected: the field keeps the old text
                    }
                }
                let tab = t.id;
                if let Some(s) = &mut self.search {
                    s.text = text;
                }
                return self.reveal(side, tab);
            }
            Message::SearchSubmit => {
                if let Some(s) = self.search.take()
                    && !s.filter
                {
                    return self.act(s.side, Action::Enter);
                }
            }
```
- Key gate in `Message::Key`, after the dialog and job checks and before `return self.act(self.active, action);`:
```rust
                if let Some(s) = &self.search {
                    let (side, filter, text) = (s.side, s.filter, s.text.clone());
                    match action {
                        Action::Up | Action::Down if !filter => {
                            let p = &self.panes[side].active().panel;
                            let n = p.entries().len();
                            let from = if action == Action::Down { p.cursor() + 1 } else { p.cursor() + n - 1 };
                            if let Some(i) = p.find(&text, from, action == Action::Down) {
                                let t = self.panes[side].active_mut();
                                t.panel.set_cursor(i);
                                let tab = t.id;
                                return self.reveal(side, tab);
                            }
                            return Task::none();
                        }
                        Action::Up | Action::Down => {} // filter: plain cursor move below, field stays
                        _ => self.search = None,       // any other key closes the field and acts
                    }
                }
```
- Escape: replace the `None => self.cancel_job(),` arm of `Message::DialogCancel` with:
```rust
                None => {
                    if let Some(s) = self.search.take() {
                        if s.filter {
                            self.panes[s.side].active_mut().panel.set_filter(None);
                        }
                    } else if self.job.is_some() {
                        self.cancel_job();
                    } else {
                        self.panes[self.active].active_mut().panel.set_filter(None);
                    }
                }
```
- Mouse closes the field: at the top of the `Click`, `DoubleClick`, `Header`, `SelectTab`, `CloseTabAt` and `Drive` arms, add `self.search = None;`.

`view.rs`, in `pane()`, replace the `status` construction with:
```rust
    let status: Element<_> = match (&app.search, &p.error) {
        (Some(s), _) if s.side == side => {
            let label = if s.filter { fl!("filter-label") } else { fl!("search-label") };
            row![
                text(label).size(TEXT),
                widget::text_input("", &s.text)
                    .id(app.input_id.clone())
                    .on_input(Message::SearchInput)
                    .on_submit(|_| Message::SearchSubmit)
                    .size(TEXT)
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
        (_, Some(e)) => /* unchanged error text */,
        (_, None) => {
            let (t, m) = (p.panel.totals(), p.panel.marked_totals());
            let totals = fl!("status", /* unchanged args */);
            let line = match p.panel.filter() {
                Some(f) => format!("{}  {totals}", fl!("filter-status", pattern = f)),
                None => totals,
            };
            text(line).size(TEXT).into()
        }
    };
```
Keep the existing error branch body as is. Move the existing `fl!("status", …)` call into `totals`.

ftl `en`:
```
search-label = Search:
filter-label = Filter:
filter-status = Filter: { $pattern } ·
```
ftl `ru`:
```
search-label = Поиск:
filter-label = Фильтр:
filter-status = Фильтр: { $pattern } ·
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Smoke run**

`cargo run -p shagoff-commander` for a few seconds with a timeout. Check that it starts without panics (no keys are sent).

- [ ] **Step 6: Commit**

```bash
git add crates/app && git commit -m "app: quick search field and Ctrl+S filter"
```

---

### Task 5: docs

**Files:**
- Modify: `.claude/docs/tc-reference.md`, `TESTING.md`, `.claude/docs/ROADMAP.md`

- [ ] **Step 1: tc-reference**
  - Change the `Alt+буквы` row to phase `07`, with the note: «Поле внизу панели; префикс, `*` — любое место; ↑/↓ — следующее/предыдущее; Enter открывает; другая клавиша закрывает и выполняется».
  - Add a row `| Ctrl+S | Быстрый фильтр панели | 07 | То же поле; Enter оставляет фильтр, Escape снимает |`.
- [ ] **Step 2: TESTING.md** — add a section «Быстрый поиск и фильтр (07)» with these items:
  - Alt+буква — поле внизу панели в фокусе, курсор на первом совпадении; дальше ввод без Alt; Backspace стирает
  - Буква без совпадений не принимается
  - ↑/↓ — предыдущее/следующее совпадение по кругу; Enter открывает каталог/файл; F5 из поиска — диалог копирования
  - Alt+ф в русской раскладке ищет «ф»
  - Ctrl+S — фильтр по мере ввода; Enter — поле закрыто, «Фильтр: …» в строке состояния; Escape снимает; смена каталога снимает
  - Отметки на скрытых фильтром файлах сняты; автообновление при фильтре показывает новые подходящие файлы
  - Клик мышью по панели закрывает поле
- [ ] **Step 3: ROADMAP** — tick «Быстрый поиск Alt+буквы и фильтр панели» in the backlog (`- [x]` and `(сделано, 07)`).
- [ ] **Step 4: verify + commit**

```bash
just verify && git add -A TESTING.md .claude && git commit -m "docs: quick search in tc-reference, TESTING, ROADMAP"
```
