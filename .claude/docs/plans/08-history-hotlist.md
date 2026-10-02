# History, Hotlist, Swap Panes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-tab directory history (Alt+←/→, and Alt+↓ as a list), a hotlist of favourite dirs (Ctrl+D) stored in the config, and Ctrl+U to swap the panels.

**Architecture:**
- `core::history::History` is a pure data structure kept on every `Tab`.
- One `Dialog::List { kind, side, cursor, items }` replaces `Dialog::Drives` and serves the drives, history and hotlist lists.
- `Config.hotlist` holds the hotlist and is written through the generated `set_hotlist`.
- `Message::Listed` finds its tab by id in both panes, so scans survive Ctrl+U.

**Tech Stack:** Rust 2024, libcosmic rev `ef490df50b0a05a21c494c3f75737581bf0b39d9`, cosmic-config.

**Spec:** `.claude/docs/specs/08-history-hotlist.md`

## Global Constraints

- `crates/core` must not depend on libcosmic. History logic lives in core, with tests.
- Every new UI string goes in both `en` and `ru` ftl.
- Letter shortcuts match physical keys (`Code::KeyD`, `Code::KeyU`). Modifiers must match exactly; the Alt block requires `mods == Modifiers::ALT`.
- New config fields have defaults (`hotlist` = empty), so old installs keep loading.
- Tests must not write the real config: `App::build` leaves `config_handler` as `None`.
- `just verify` must pass before each task's commit.

## Review Focus

1. **A scan in flight during Ctrl+U.** The result must land in its tab, now on the other side, and must not leave the tab stuck in `pending`. Pinned by `regression_swap_keeps_scan_in_flight` (Task 4).
2. **Rescans and failed navigations must not pollute the history.** The watcher, Ctrl+R and the same path add nothing. Pinned by `rescan_does_not_add_history` (Task 4).
3. **Hotlist "add" twice and delete at the edges.** Adding the same dir again does nothing. Deleting the last entry clamps the cursor, and the add row can't be deleted. Pinned by `hotlist_add_once_delete_clamps` (Task 5).
4. **Drive behaviour unchanged after the move to `Dialog::List`.** The snapshot, cursor on the current drive, Enter and click all still work. Pinned by the existing drive tests, adapted (Task 3).
5. **History at its limit.** At 51 entries the oldest one goes and back/forward still land correctly. Pinned by `history::tests::limit_drops_oldest_keeps_position` (Task 1).

---

### Task 1: `core::history`

**Files:**
- Create: `crates/core/src/history.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod history;`)

**Interfaces:**
- Produces: `history::{History, LIMIT}`, with `History::{visit(&mut self, &Path), back(&mut self) -> Option<PathBuf>, forward(&mut self) -> Option<PathBuf>, recent(&self) -> Vec<PathBuf>}`. `History` derives `Clone, Debug, Default`.

- [ ] **Step 1: Failing tests** (file with stubs `todo!()`)

```rust
//! Per-tab directory history for Alt+← / Alt+→ / Alt+↓.

use std::path::{Path, PathBuf};

pub const LIMIT: usize = 50;

#[derive(Clone, Debug, Default)]
pub struct History {
    items: Vec<PathBuf>,
    pos: usize,
}

impl History {
    pub fn visit(&mut self, _path: &Path) { todo!() }
    pub fn back(&mut self) -> Option<PathBuf> { todo!() }
    pub fn forward(&mut self) -> Option<PathBuf> { todo!() }
    pub fn recent(&self) -> Vec<PathBuf> { todo!() }
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
    fn recent_is_unique_newest_first() {
        let h = h(&["/a", "/b", "/a", "/c"]);
        assert_eq!(h.recent(), [PathBuf::from("/c"), "/a".into(), "/b".into()]);
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p shagoff-core history`
Expected: panics on `todo!()`.

- [ ] **Step 3: Implement**

```rust
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
        if self.pos == 0 || self.items.is_empty() {
            return None;
        }
        self.pos -= 1;
        Some(self.items[self.pos].clone())
    }

    pub fn forward(&mut self) -> Option<PathBuf> {
        if self.pos + 1 >= self.items.len() {
            return None;
        }
        self.pos += 1;
        Some(self.items[self.pos].clone())
    }

    /// Unique dirs, most recent visit first.
    pub fn recent(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in self.items.iter().rev() {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }
}
```
`recent` with `items` [/a,/b,/a,/c] gives [/c,/a,/b], which matches the test. Note that `recent` reflects visit order, not `pos`. That is fine: TC lists recent dirs.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-core history && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/core && git commit -m "core: per-tab directory history"
```

---

### Task 2: keymap — history, hotlist, swap

**Files:**
- Modify: `crates/app/src/keymap.rs`, `crates/app/src/app.rs` (a placeholder arm only)

**Interfaces:**
- Produces: `Action::{HistoryBack, HistoryForward, HistoryList, Hotlist, SwapPanes}`

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn alt_arrows_walk_history() {
        assert_eq!(named(Named::ArrowLeft, ALT), Some(Action::HistoryBack));
        assert_eq!(named(Named::ArrowRight, ALT), Some(Action::HistoryForward));
        assert_eq!(named(Named::ArrowDown, ALT), Some(Action::HistoryList));
        assert_eq!(named(Named::ArrowLeft, ALT | Modifiers::SHIFT), None);
    }

    #[test]
    fn ctrl_d_hotlist_ctrl_u_swap() {
        assert_eq!(chr("d", Code::KeyD, CTRL), Some(Action::Hotlist));
        assert_eq!(chr("в", Code::KeyD, CTRL), Some(Action::Hotlist));
        assert_eq!(chr("u", Code::KeyU, CTRL), Some(Action::SwapPanes));
        assert_eq!(chr("г", Code::KeyU, CTRL), Some(Action::SwapPanes));
    }
```

- [ ] **Step 2: Run — expect FAIL** (no variants)

Run: `cargo test -p shagoff-commander keymap`

- [ ] **Step 3: Implement**

Add variants with doc comments. In the Alt block `match key`, add before `Key::Character`:
```rust
            Key::Named(Named::ArrowLeft) if mods == Modifiers::ALT => Some(Action::HistoryBack),
            Key::Named(Named::ArrowRight) if mods == Modifiers::ALT => Some(Action::HistoryForward),
            Key::Named(Named::ArrowDown) if mods == Modifiers::ALT => Some(Action::HistoryList),
```
Physical table:
```rust
        (Physical::Code(Code::KeyD), true, false) => Some(Action::Hotlist),
        (Physical::Code(Code::KeyU), true, false) => Some(Action::SwapPanes),
```
`app.rs` `act()`: temporary arm `Action::HistoryBack | Action::HistoryForward | Action::HistoryList | Action::Hotlist | Action::SwapPanes => {}`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "keymap: Alt+arrows history, Ctrl+D hotlist, Ctrl+U swap"
```

---

### Task 3: `Dialog::List` replaces `Dialog::Drives`

**Files:**
- Modify: `crates/app/src/dialogs.rs`, `crates/app/src/app.rs`, both ftl files

**Interfaces:**
- Produces:
  - `dialogs::{ListKind { Drives, History, Hotlist }, ListItem { pub label: String, pub path: PathBuf }}`;
  - `Dialog::List { kind: ListKind, side: usize, cursor: usize, items: Vec<ListItem> }`;
  - `Message::ListPick(usize)`;
  - `App::go_to(&mut self, side: usize, path: PathBuf) -> Task<Message>` (renamed from `go_drive`);
  - `App::pick(&mut self, i: usize) -> Task<Message>`, which acts on list entry `i` of the open list.

- [ ] **Step 1: Adapt the existing drive tests first** (these are the spec for behaviour preservation)

In `drive_dialog_navigates_and_opens` and `regression_drive_list_is_fixed_while_dialog_open`, replace every `Dialog::Drives { side: 1, .. }` / `{ cursor: N, .. }` pattern with `Dialog::List { kind: ListKind::Drives, side: 1, .. }` / `Dialog::List { cursor: N, .. }`. Add a test:
```rust
    #[test]
    fn clicking_a_list_entry_opens_it() {
        let mut app = app_with(Config::default(), State::default());
        app.drives = vec![
            Drive { label: "/".into(), path: "/".into() },
            Drive { label: "etc".into(), path: "/etc".into() },
        ];
        let _ = app.update(Message::Key(Action::Drives(1)));
        app.panes[1].active_mut().pending = None;
        let _ = app.update(Message::ListPick(1));
        assert!(app.dialog.is_none());
        let pending = app.panes[1].active().pending.as_ref().map(|(_, p)| p.clone());
        assert_eq!(pending, Some(PathBuf::from("/etc")));
    }
```
Imports in tests: `use crate::dialogs::ListKind;`.

- [ ] **Step 2: Run — expect FAIL** (compile: no `Dialog::List`, `ListPick`)

Run: `cargo test -p shagoff-commander`

- [ ] **Step 3: Implement**

`dialogs.rs`:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Drives,
    History,
    Hotlist,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub label: String,
    pub path: PathBuf,
}
```
Replace the `Drives { side, cursor, drives }` variant with:
```rust
    /// Alt+F1/F2, Alt+↓, Ctrl+D: a snapshot taken when the list opened.
    List {
        kind: ListKind,
        side: usize,
        cursor: usize,
        items: Vec<ListItem>,
    },
```
The view arm replaces the Drives arm:
```rust
        Dialog::List { kind, cursor, items, .. } => {
            let mut list = column![].spacing(2);
            for (i, item) in items.iter().enumerate() {
                let label = if item.path.as_os_str().is_empty() {
                    item.label.clone() // the hotlist's "add current dir" row
                } else {
                    format!("{}   {}", item.label, item.path.display())
                };
                let b = if i == *cursor {
                    widget::button::suggested(label)
                } else {
                    widget::button::text(label)
                };
                list = list.push(b.on_press(Message::ListPick(i)).width(cosmic::iced::Length::Fill));
            }
            let title = match kind {
                ListKind::Drives => fl!("drives"),
                ListKind::History => fl!("history"),
                ListKind::Hotlist => fl!("hotlist"),
            };
            widget::dialog().title(title).control(list).secondary_action(cancel).into()
        }
```
Remove the now-unused `Drive` import from `dialogs.rs` if nothing else uses it.

`app.rs`:
- The `Message::Drive(side, path)` variant stays for the drive bar buttons. Add `ListPick(usize)` with the doc comment `/// Click on entry i of the open list dialog.`.
- `dialog_for`, Drives:
```rust
            Action::Drives(s) => Some(Dialog::List {
                kind: ListKind::Drives,
                side: s,
                cursor: drives::containing(&self.drives, self.panes[s].active().panel.cwd()).unwrap_or(0),
                items: self.drives.iter().map(|d| ListItem { label: d.label.clone(), path: d.path.clone() }).collect(),
            }),
```
- Key gate: replace the `if let Some(Dialog::Drives { side, cursor, drives }) = &mut self.dialog { … }` block with:
```rust
                if let Some(Dialog::List { cursor, items, .. }) = &mut self.dialog {
                    match action {
                        Action::Up => *cursor = cursor.saturating_sub(1),
                        Action::Down => *cursor = (*cursor + 1).min(items.len().saturating_sub(1)),
                        Action::Enter => {
                            let i = *cursor;
                            return self.pick(i);
                        }
                        _ => {}
                    }
                    return Task::none();
                }
```
- `Message::ListPick(i) => return self.pick(i),`
- `submit_dialog`: `Dialog::List { cursor, .. }` → put the dialog back and call `self.pick(cursor)`. Simplest form: `d @ Dialog::List { .. } => { let i = match &d { Dialog::List { cursor, .. } => *cursor, _ => 0 }; self.dialog = Some(d); self.pick(i) }`.
- Methods:
```rust
    /// Enter / click on entry `i` of the open list.
    fn pick(&mut self, i: usize) -> Task<Message> {
        let Some(Dialog::List { side, items, .. }) = self.dialog.take() else {
            return Task::none();
        };
        match items.get(i) {
            Some(item) => self.go_to(side, item.path.clone()),
            None => Task::none(),
        }
    }
```
- Rename `go_drive` → `go_to` everywhere (`Message::Drive` arm included).

ftl: `history = History` / `История`; `hotlist = Hotlist` / `Избранное` (en/ru).

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "app: one list dialog for drives (history and hotlist next)"
```

---

### Task 4: history in tabs, Alt+←/→/↓, Ctrl+U, Listed routed by tab id

**Files:**
- Modify: `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `History` (Task 1), the `Action`s (Task 2), `Dialog::List`/`ListKind::History` (Task 3)
- Produces: `Tab.history: History` (pub(crate))

- [ ] **Step 1: Failing tests**

```rust
    /// Simulate a finished scan of `path` for the active tab of `side`.
    fn arrive(app: &mut App, side: usize, path: &Path) {
        let _ = app.load(side, path.into(), None);
        let t = app.panes[side].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            side,
            tab: id,
            generation,
            path: path.into(),
            focus: None,
            result: Ok(vec![]),
            space: None,
        });
    }

    fn pending_of(app: &App, side: usize) -> Option<PathBuf> {
        app.panes[side].active().pending.as_ref().map(|(_, p)| p.clone())
    }

    #[test]
    fn alt_left_right_walk_tab_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/etc"));
        let _ = app.update(Message::Key(Action::HistoryBack));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/usr")));
        arrive(&mut app, 0, Path::new("/usr")); // the jump lands
        let _ = app.update(Message::Key(Action::HistoryForward));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
    }

    #[test]
    fn rescan_does_not_add_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/usr")); // Ctrl+R / watcher
        let recent = app.panes[0].active().history.recent();
        assert_eq!(recent.iter().filter(|p| p.as_path() == Path::new("/usr")).count(), 1);
    }

    #[test]
    fn alt_down_lists_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/etc"));
        let _ = app.update(Message::Key(Action::HistoryList));
        let Some(Dialog::List { kind: ListKind::History, items, .. }) = &app.dialog else {
            panic!("no history list: {:?}", app.dialog.is_some());
        };
        assert_eq!(items[0].path, Path::new("/etc"));
        assert!(items.iter().any(|i| i.path == Path::new("/usr")));
    }

    #[test]
    fn ctrl_u_swaps_panes() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 1, Path::new("/etc"));
        let _ = app.update(Message::Key(Action::SwapPanes));
        assert_eq!(app.panes[0].active().panel.cwd(), Path::new("/etc"));
        assert_eq!(app.panes[1].active().panel.cwd(), Path::new("/usr"));
    }

    #[test]
    fn regression_swap_keeps_scan_in_flight() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.load(0, "/usr".into(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Key(Action::SwapPanes)); // tab now on side 1
        let _ = app.update(Message::Listed {
            side: 0, // side when the scan started
            tab: id,
            generation,
            path: "/usr".into(),
            focus: None,
            result: Ok(vec![]),
            space: None,
        });
        let t = app.panes[1].active();
        assert_eq!(t.panel.cwd(), Path::new("/usr"));
        assert!(t.pending.is_none(), "tab stuck in pending");
    }
```

- [ ] **Step 2: Run — expect FAIL** (compile: no `history` field; then behaviour)

Run: `cargo test -p shagoff-commander`

- [ ] **Step 3: Implement**

- `Tab`: add `pub(crate) history: History` (default in `Tab::new`; `duplicate` clones it: `history: self.history.clone()`). Import `use shagoff_core::history::History;`.
- `Message::Listed` arm, at the top, re-route by tab id:
```rust
                // The tab may have moved (Ctrl+U): find it by id, the side is only where it started.
                let Some(side) = (0..2).find(|&s| self.panes[s].items().iter().any(|t| t.id == tab)) else {
                    return Task::none(); // tab was closed
                };
```
  Then keep the existing `find(|t| t.id == tab)` lookup (its `else` is now unreachable but harmless; keep the early return).
- In the `Ok(entries)` branch, before `set_listing`:
```rust
                        if path != t.panel.cwd() || t.history.recent().is_empty() {
                            t.history.visit(&path);
                        }
```
  (`visit` is a no-op for the current entry anyway; the condition keeps rescans cheap and documents intent.)
- `act()` arms (replace the temporary arm):
```rust
            Action::HistoryBack | Action::HistoryForward => {
                let step = if action == Action::HistoryBack { t.history.back() } else { t.history.forward() };
                if let Some(path) = step {
                    return self.load(side, path, None);
                }
            }
            Action::SwapPanes => {
                self.panes.swap(0, 1);
                self.search = None;
                return Task::batch([self.restore_scroll(0), self.restore_scroll(1)]);
            }
            // Opened by `dialog_for` above.
            Action::HistoryList | Action::Hotlist => {}
```
  `t` is the `active_mut()` borrow taken before the match; use it before `panel` is used (or take `self.panes[side].active_mut().history.back()` directly).
- `dialog_for`:
```rust
            Action::HistoryList => Some(Dialog::List {
                kind: ListKind::History,
                side,
                cursor: 0,
                items: self.panes[side]
                    .active()
                    .history
                    .recent()
                    .into_iter()
                    .map(|p| ListItem { label: format::dir_title(&p), path: p })
                    .collect(),
            }),
```
  (`use shagoff_core::format;` if it isn't imported in app.rs; `dir_title` gives the last component, or `/`.)

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "app: per-tab history (Alt+arrows, Alt+Down list), Ctrl+U swaps panes"
```

---

### Task 5: hotlist (Ctrl+D)

**Files:**
- Modify: `crates/app/src/config.rs`, `crates/app/src/app.rs`, both ftl files

**Interfaces:**
- Consumes: `Dialog::List`/`ListKind::Hotlist`, `pick` (Task 3)
- Produces:
  - `config::HotEntry { pub name: String, pub path: PathBuf }` (Clone, Debug, PartialEq, Eq, Serialize, Deserialize);
  - `Config.hotlist: Vec<HotEntry>`.

- [ ] **Step 1: Failing tests**

```rust
    fn hotlist_items(app: &App) -> Vec<PathBuf> {
        match &app.dialog {
            Some(Dialog::List { kind: ListKind::Hotlist, items, .. }) => items.iter().map(|i| i.path.clone()).collect(),
            _ => panic!("hotlist not open"),
        }
    }

    #[test]
    fn hotlist_add_once_delete_clamps() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // row 0: add current dir
        assert_eq!(app.config.hotlist.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), [cwd.clone()]);
        assert!(app.dialog.is_none());

        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // add again: no duplicate
        assert_eq!(app.config.hotlist.len(), 1);

        let _ = app.update(Message::Key(Action::Hotlist));
        assert_eq!(hotlist_items(&app).len(), 2); // add row + one entry
        let _ = app.update(Message::Key(Action::Delete)); // on the add row: nothing
        assert_eq!(app.config.hotlist.len(), 1);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Delete)); // remove the entry
        assert!(app.config.hotlist.is_empty());
        assert_eq!(hotlist_items(&app).len(), 1); // still open, cursor clamped to the add row
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
    }

    #[test]
    fn hotlist_entry_opens_its_dir() {
        let config = Config {
            hotlist: vec![crate::config::HotEntry { name: "etc".into(), path: "/etc".into() }],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Down));
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::Enter));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
    }
```

- [ ] **Step 2: Run — expect FAIL** (no `hotlist` field)

Run: `cargo test -p shagoff-commander`

- [ ] **Step 3: Implement**

`config.rs`:
```rust
/// A favourite dir (Ctrl+D).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotEntry {
    pub name: String,
    pub path: PathBuf,
}
```
Add `pub hotlist: Vec<HotEntry>,` to `Config` (with the doc comment `/// Ctrl+D favourites, in menu order.`) and `hotlist: Vec::new()` to `Default`.

`app.rs`:
- `dialog_for`: `Action::Hotlist => Some(Dialog::List { kind: ListKind::Hotlist, side, cursor: 0, items: self.hotlist_items() })`.
- Method:
```rust
    /// Hotlist rows: "add current dir" (empty path), then the favourites.
    fn hotlist_items(&self) -> Vec<ListItem> {
        std::iter::once(ListItem { label: fl!("hotlist-add"), path: PathBuf::new() })
            .chain(self.config.hotlist.iter().map(|e| ListItem { label: e.name.clone(), path: e.path.clone() }))
            .collect()
    }

    fn save_hotlist(&mut self, list: Vec<HotEntry>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_hotlist(h, list) {
                    log::warn!("config: {e}");
                }
            }
            None => self.config.hotlist = list,
        }
    }
```
- In `pick`, before `go_to`: for `ListKind::Hotlist` with `i == 0`, add the current dir:
```rust
        if kind == ListKind::Hotlist && i == 0 {
            let cwd = self.panes[side].active().panel.cwd().to_path_buf();
            if !self.config.hotlist.iter().any(|e| e.path == cwd) {
                let mut list = self.config.hotlist.clone();
                list.push(HotEntry { name: format::dir_title(&cwd), path: cwd });
                self.save_hotlist(list);
            }
            return Task::none();
        }
```
  (Destructure `kind` in `pick`'s `let Some(Dialog::List { kind, side, items, .. })`.)
- Key gate for `Dialog::List`: add an arm for Delete in the hotlist:
```rust
                        Action::Delete | Action::DeletePermanent => {
                            // handled below: needs &mut self, so leave the borrow first
                        }
```
  Restructure: compute `(kind, cursor)` from the dialog first. If `kind == ListKind::Hotlist && cursor >= 1 && matches!(action, Action::Delete | Action::DeletePermanent)`:
```rust
                    let mut list = self.config.hotlist.clone();
                    if cursor - 1 < list.len() {
                        list.remove(cursor - 1);
                        self.save_hotlist(list);
                    }
                    let items = self.hotlist_items();
                    if let Some(Dialog::List { items: it, cursor: c, .. }) = &mut self.dialog {
                        *c = (*c).min(items.len() - 1);
                        *it = items;
                    }
                    return Task::none();
```
- Message::Config already replaces `self.config`, so hand edits to `hotlist` apply on the next Ctrl+D.

ftl: `hotlist-add = Add current directory` / `Добавить текущий каталог`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "app: hotlist (Ctrl+D) stored in config"
```

---

### Task 6: docs

- [ ] **Step 1: tc-reference**
  - Rows `Alt+← / Alt+→` and `Ctrl+U`: phase `08`.
  - Add the row `| Alt+↓ | Список истории каталогов вкладки | 08 | |`.
  - Row `Ctrl+D`: phase `08`, with the note «Первый пункт — добавить текущий; Delete удаляет; хранится в конфиге `hotlist`».
- [ ] **Step 2: TESTING.md** — add a section «История, избранное, обмен панелей (08)» with these items:
  - Alt+←/→ ходят по каталогам вкладки; Ctrl+R и автообновление не добавляют записей; у новой вкладки (Ctrl+T) та же история
  - Alt+↓ — список свежих каталогов, Enter переходит
  - Ctrl+D — «Добавить текущий каталог», повтор не дублирует; Enter на пункте — переход; Delete удаляет; после перезапуска список на месте; правка `hotlist` в файле конфига видна при следующем Ctrl+D
  - Ctrl+U меняет панели вместе с вкладками; фокус на той же стороне
  - Alt+F1/F2 работают как раньше

  Add the row `regression_swap_keeps_scan_in_flight` to the regression table.
- [ ] **Step 3: ROADMAP** — tick the backlog line «История Alt+←/→, hotlist Ctrl+D, Ctrl+U (поменять панели)» with «(сделано, 08)».
- [ ] **Step 4: verify + commit**

```bash
just verify && git add -A TESTING.md .claude && git commit -m "docs: history, hotlist, swap in tc-reference, TESTING, ROADMAP"
```
