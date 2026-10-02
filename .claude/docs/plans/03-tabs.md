# Фаза 3: вкладки — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each pane gets TC-style tabs (Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab, a tab bar shown when there are 2+ tabs), and the keymap matches modifiers exactly.

**Architecture:**
- A generic `Tabs<T>` (never empty, one active tab) lives in core and is unit-tested.
- In the app, the old `Pane` becomes `Tab`, with a stable `id`. Each pane is a `Tabs<Tab>`.
- Scan results are routed by tab id.

**Tech Stack:** Rust 2024, libcosmic (pinned rev).

**Spec:** `.claude/docs/specs/2026-10-02-tabs.md`

## Global Constraints

- `crates/core` has no UI dependencies. No new crates.
- Every UI string goes through `fl!` in both `en` and `ru`. (This phase adds no new strings: tab titles are directory names.)
- Ctrl+T and Ctrl+W match the physical key (`Code::KeyT`, `Code::KeyW`).
- Modifiers must match exactly. Alt or Super with any key gives `None`.
- `just verify` must be clean. Branch: `feat/tabs`.

## Review Focus

1. **Scan finishes after its tab was closed, or after the user switched tabs.** If the tab is closed, the result is dropped. If the tab still exists but isn't active, the result is applied to it, and the active tab is left untouched. Covered by the id routing in Task 3 and the manual check.
2. **Ctrl+W on the only tab.** Nothing happens and there is no panic. Tested by `close_refuses_last` in Task 1.
3. **Closing the active tab when it is the rightmost one.** The left neighbor becomes active, and the index stays in bounds. Tested by `close_active_rightmost_selects_left` in Task 1.
4. **Switching tabs restores each tab's own scroll position.** No blank list and no cursor off-screen. Covered by `restore_scroll` in Task 3 and the manual check on `/usr/lib` against `~`.
5. **The compositor may grab Ctrl+Tab.** If it does, record it in tc-reference.md as a note. Manual check.

---

### Task 1: core — `Tabs<T>`, `Panel: Clone`, `dir_title`

**Files:**
- Create: `crates/core/src/tabs.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod tabs;`)
- Modify: `crates/core/src/panel.rs` (`#[derive(Debug, Clone)]` on `Panel`)
- Modify: `crates/core/src/format.rs` (`dir_title`)

**Interfaces:**
- Produces:
  - `Tabs<T>` with:
    - `new(T) -> Self`
    - `items(&self) -> &[T]`, `items_mut(&mut self) -> &mut [T]`
    - `active_index(&self) -> usize`, `active(&self) -> &T`, `active_mut(&mut self) -> &mut T`
    - `open_after(&mut self, T)`
    - `close(&mut self, usize) -> bool`
    - `select(&mut self, usize)`
    - `next(&mut self)`, `prev(&mut self)`
  - `format::dir_title(&Path) -> String`

- [ ] **Step 1: Write the failing tests**

  Add `pub mod tabs;` to `lib.rs`. Then create `crates/core/src/tabs.rs`:
  ```rust
  //! Ordered tabs with exactly one active; never empty.

  #[derive(Debug, Clone)]
  pub struct Tabs<T> {
      items: Vec<T>,
      active: usize,
  }

  impl<T> Tabs<T> {
      pub fn new(first: T) -> Self {
          Self { items: vec![first], active: 0 }
      }
      pub fn items(&self) -> &[T] {
          &self.items
      }
      pub fn items_mut(&mut self) -> &mut [T] {
          &mut self.items
      }
      pub fn active_index(&self) -> usize {
          self.active
      }
      pub fn active(&self) -> &T {
          &self.items[self.active]
      }
      pub fn active_mut(&mut self) -> &mut T {
          &mut self.items[self.active]
      }
      pub fn open_after(&mut self, _item: T) {
          todo!()
      }
      pub fn close(&mut self, _i: usize) -> bool {
          todo!()
      }
      pub fn select(&mut self, _i: usize) {
          todo!()
      }
      pub fn next(&mut self) {
          todo!()
      }
      pub fn prev(&mut self) {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      /// Tabs "a".."=n" with `active` selected.
      fn tabs(n: usize, active: usize) -> Tabs<char> {
          let mut t = Tabs::new('a');
          for c in (1..n).map(|i| (b'a' + i as u8) as char) {
              t.items.push(c);
          }
          t.active = active;
          t
      }

      #[test]
      fn open_after_inserts_next_to_active_and_activates() {
          let mut t = tabs(3, 0); // [a] b c
          t.open_after('x');
          assert_eq!(t.items(), ['a', 'x', 'b', 'c']);
          assert_eq!(*t.active(), 'x');
      }

      #[test]
      fn close_refuses_last() {
          let mut t = Tabs::new('a');
          assert!(!t.close(0));
          assert_eq!(t.items(), ['a']);
      }

      #[test]
      fn close_out_of_range_is_noop() {
          let mut t = tabs(2, 0);
          assert!(!t.close(5));
          assert_eq!(t.items().len(), 2);
      }

      #[test]
      fn close_active_middle_selects_right() {
          let mut t = tabs(3, 1); // a [b] c
          assert!(t.close(1));
          assert_eq!(t.items(), ['a', 'c']);
          assert_eq!(*t.active(), 'c');
      }

      #[test]
      fn close_active_rightmost_selects_left() {
          let mut t = tabs(3, 2); // a b [c]
          assert!(t.close(2));
          assert_eq!(*t.active(), 'b');
      }

      #[test]
      fn close_left_of_active_keeps_active_tab() {
          let mut t = tabs(3, 2); // a b [c]
          assert!(t.close(0));
          assert_eq!(*t.active(), 'c');
      }

      #[test]
      fn close_right_of_active_keeps_active_tab() {
          let mut t = tabs(3, 0); // [a] b c
          assert!(t.close(2));
          assert_eq!(*t.active(), 'a');
      }

      #[test]
      fn select_ignores_out_of_range() {
          let mut t = tabs(3, 0);
          t.select(2);
          assert_eq!(*t.active(), 'c');
          t.select(9);
          assert_eq!(*t.active(), 'c');
      }

      #[test]
      fn next_prev_wrap() {
          let mut t = tabs(3, 2);
          t.next();
          assert_eq!(*t.active(), 'a');
          t.prev();
          assert_eq!(*t.active(), 'c');
          let mut one = Tabs::new('a');
          one.next();
          one.prev();
          assert_eq!(*one.active(), 'a');
      }
  }
  ```

  In `format.rs`, add a stub `pub fn dir_title(_p: &std::path::Path) -> String { todo!() }` and this test:
  ```rust
  #[test]
  fn dir_title_last_component_or_root() {
      use std::path::Path;
      assert_eq!(dir_title(Path::new("/home/shag")), "shag");
      assert_eq!(dir_title(Path::new("/")), "/");
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib`

  Expected: 10 tests fail with `not yet implemented` (9 in `tabs`, 1 `dir_title`). The other 36 pass.

- [ ] **Step 3: Implement**

  In tabs.rs:
  ```rust
      /// Insert right after the active tab and activate it.
      pub fn open_after(&mut self, item: T) {
          self.active += 1;
          self.items.insert(self.active, item);
      }

      /// Close tab `i`; refuses the last one. Closing the active tab activates its right neighbour (left if none).
      pub fn close(&mut self, i: usize) -> bool {
          if self.items.len() == 1 || i >= self.items.len() {
              return false;
          }
          self.items.remove(i);
          if i < self.active || self.active == self.items.len() {
              self.active -= 1;
          }
          true
      }

      pub fn select(&mut self, i: usize) {
          if i < self.items.len() {
              self.active = i;
          }
      }

      pub fn next(&mut self) {
          self.active = (self.active + 1) % self.items.len();
      }

      pub fn prev(&mut self) {
          self.active = (self.active + self.items.len() - 1) % self.items.len();
      }
  ```

  In format.rs:
  ```rust
  /// Tab title: the last path component, `/` for the root.
  pub fn dir_title(p: &std::path::Path) -> String {
      p.file_name()
          .map_or_else(|| "/".to_string(), |n| n.to_string_lossy().into_owned())
  }
  ```

  In panel.rs, change `#[derive(Debug)]` on `Panel` to `#[derive(Debug, Clone)]`.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib`

  Expected: 46 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core && git commit -m "core: Tabs<T>, dir_title, Panel: Clone"
  ```

---

### Task 2: keymap — tab keys and exact modifiers

**Files:**
- Modify: `crates/app/src/keymap.rs`

**Interfaces:**
- Produces: the `Action` enum gains the variants `NewTab`, `CloseTab`, `NextTab` and `PrevTab`. The signature of `action(&Key, Physical, Modifiers)` stays the same.

- [ ] **Step 1: Write the failing tests**

  Add the four variants to `Action`, right after `Sort(SortKey)`. Then add these tests:
  ```rust
  #[test]
  fn tab_keys() {
      const CTRL_SHIFT: Modifiers = Modifiers::CTRL.union(Modifiers::SHIFT);
      assert_eq!(named(Named::Tab, CTRL), Some(Action::NextTab));
      assert_eq!(named(Named::Tab, CTRL_SHIFT), Some(Action::PrevTab));
      assert_eq!(chr("t", Code::KeyT, CTRL), Some(Action::NewTab));
      assert_eq!(chr("е", Code::KeyT, CTRL), Some(Action::NewTab)); // Russian layout
      assert_eq!(chr("w", Code::KeyW, CTRL), Some(Action::CloseTab));
      assert_eq!(chr("ц", Code::KeyW, CTRL), Some(Action::CloseTab));
  }

  #[test]
  fn modifiers_must_match_exactly() {
      const SHIFT: Modifiers = Modifiers::SHIFT;
      const ALT: Modifiers = Modifiers::ALT;
      assert_eq!(named(Named::Tab, SHIFT), None);
      assert_eq!(named(Named::Enter, ALT), None);
      assert_eq!(named(Named::ArrowDown, SHIFT), None);
      assert_eq!(named(Named::F3, CTRL.union(ALT)), None);
      assert_eq!(chr("R", Code::KeyR, CTRL.union(SHIFT)), None);
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 2 failures. `tab_keys` fails because Ctrl+Tab gives `None`. `modifiers_must_match_exactly` fails because Shift+Tab gives `Some(SwitchPane)`.

- [ ] **Step 3: Implement**

  Replace the body of `action`:
  ```rust
  pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
      use cosmic::iced::keyboard::key::{Code, Named};
      if mods.alt() || mods.logo() {
          return None;
      }
      let (ctrl, shift) = (mods.control(), mods.shift());
      if let Key::Named(n) = key {
          return Some(match (n, ctrl, shift) {
              (Named::Tab, false, false) => Action::SwitchPane,
              (Named::Tab, true, false) => Action::NextTab,
              (Named::Tab, true, true) => Action::PrevTab,
              (Named::ArrowUp, false, false) => Action::Up,
              (Named::ArrowDown, false, false) => Action::Down,
              (Named::PageUp, false, false) => Action::PageUp,
              (Named::PageDown, false, false) => Action::PageDown,
              (Named::Home, false, false) => Action::Home,
              (Named::End, false, false) => Action::End,
              (Named::Enter, false, false) => Action::Enter,
              (Named::Backspace, false, false) | (Named::PageUp, true, false) => Action::Parent,
              (Named::F3, true, false) => Action::Sort(SortKey::Name),
              (Named::F4, true, false) => Action::Sort(SortKey::Ext),
              (Named::F5, true, false) => Action::Sort(SortKey::Date),
              (Named::F6, true, false) => Action::Sort(SortKey::Size),
              _ => return None,
          });
      }
      // Letter shortcuts by physical key so they work in any layout.
      match (physical, ctrl, shift) {
          (Physical::Code(Code::KeyR), true, false) => Some(Action::Reload),
          (Physical::Code(Code::Backslash), true, false) => Some(Action::Root),
          (Physical::Code(Code::KeyT), true, false) => Some(Action::NewTab),
          (Physical::Code(Code::KeyW), true, false) => Some(Action::CloseTab),
          _ => None,
      }
  }
  ```

  The app's `match action` won't compile until Task 3 adds the new arms. Within this task, add temporary arms `Action::NewTab | Action::CloseTab | Action::NextTab | Action::PrevTab => {}` to `App::act` so the build stays green. Task 3 replaces them.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 6 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy --all-targets -- -D warnings
  git add crates/app && git commit -m "app: tab shortcuts; keymap matches modifiers exactly"
  ```

---

### Task 3: app — panes of tabs, tab bar

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/view.rs`

**Interfaces:**
- Consumes: `Tabs<T>`, `format::dir_title` and `Panel: Clone` from Task 1; the new `Action` variants from Task 2.
- Produces:
  - `pub struct Tab { pub id: u64, pub panel, pub scroll_id, pub offset, pub height, pending, pub error }`
  - `App.panes: [Tabs<Tab>; 2]`
  - `Message::Listed` gains a `tab: u64` field
  - new messages `Message::SelectTab(usize, usize)` and `Message::CloseTabAt(usize, usize)`

This task is UI glue. The logic it relies on was tested in Tasks 1 and 2. Here it is verified by `just verify` and the manual checklist.

- [ ] **Step 1: app.rs — rename `Pane` to `Tab`, add `id`, `duplicate`, and switch to `Tabs`**
  - `struct Pane` → `struct Tab`. Add `pub id: u64` as the first field. `Tab::new(id: u64, cwd: PathBuf)`.
  - Add:
    ```rust
    /// Copy for Ctrl+T: same dir, sort, cursor and scroll; fresh id and scroll widget.
    fn duplicate(&self, id: u64) -> Self {
        Self {
            id,
            panel: self.panel.clone(),
            scroll_id: widget::Id::unique(),
            offset: self.offset,
            height: self.height,
            pending: None,
            error: None,
        }
    }
    ```
  - In `App`, replace the panes with `pub panes: [Tabs<Tab>; 2]` and add `next_id: u64`. Add the helper:
    ```rust
    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    ```
  - In `init`: `panes: [Tabs::new(Tab::new(1, left.clone())), Tabs::new(Tab::new(2, home.clone()))], next_id: 2`.
  - Add `tab: u64` to `Message::Listed`. Add `SelectTab(usize, usize)` and `CloseTabAt(usize, usize)` to `Message`.

- [ ] **Step 2: app.rs — routing**
  - In `load`, take the active tab of `side` (`self.panes[side].active_mut()`), capture `tab = t.id`, set `t.pending`, and include `tab` in `Message::Listed`.
  - In `Listed`, find the tab by id:
    ```rust
    let Some(t) = self.panes[side].items_mut().iter_mut().find(|t| t.id == tab) else {
        return Task::none(); // tab was closed
    };
    ```
    Then run the same `pending`, `Ok` and `Err` handling as before on `t`. After `Ok`, call `return self.reveal(side, tab);`.
  - Change `reveal(side)` to `reveal(side, tab_id)`, which works on the tab with that id. The scroll task targets that tab's own `scroll_id`, so it is harmless if the tab isn't on screen.
  - Add `restore_scroll(side)`. It scrolls the active tab's widget to `tab.offset`:
    ```rust
    fn restore_scroll(&self, side: usize) -> Task<Message> {
        let t = self.panes[side].active();
        scrollable::scroll_to(t.scroll_id.clone(), AbsoluteOffset { x: None, y: Some(t.offset) })
    }
    ```
  - `Click`, `DoubleClick`, `Header` and `Scrolled` act on `self.panes[side].active_mut()`.
  - In `act`, use `self.panes[side].active_mut()` wherever `self.panes[side]` was used. Call `reveal(side, active_id)`. Replace the temporary arms from Task 2 with:
    ```rust
    Action::NewTab => {
        let id = self.next_id();
        let copy = self.panes[side].active().duplicate(id);
        self.panes[side].open_after(copy);
        return self.restore_scroll(side);
    }
    Action::CloseTab => {
        let i = self.panes[side].active_index();
        self.panes[side].close(i);
        return self.restore_scroll(side);
    }
    Action::NextTab => {
        self.panes[side].next();
        return self.restore_scroll(side);
    }
    Action::PrevTab => {
        self.panes[side].prev();
        return self.restore_scroll(side);
    }
    ```
  - Add to `update`:
    ```rust
    Message::SelectTab(side, i) => {
        self.active = side;
        self.panes[side].select(i);
        return self.restore_scroll(side);
    }
    Message::CloseTabAt(side, i) => {
        self.panes[side].close(i);
        return self.restore_scroll(side);
    }
    ```

  Resolve borrow-checker conflicts mechanically: copy `id`, `page` and similar values out first, then call the `&mut self` helpers.

- [ ] **Step 3: view.rs — show the active tab and the tab bar**
  - In `pane(app, side)`, use `let tabs = &app.panes[side]; let p = tabs.active();`.
  - Add the tab bar, shown only when there are 2+ tabs, above the path line:
    ```rust
    fn tab_bar(side: usize, tabs: &Tabs<Tab>, pane_active: bool) -> Element<'_, Message> {
        let mut bar = row![].spacing(2);
        for (i, t) in tabs.items().iter().enumerate() {
            let label = container(cell(format::dir_title(t.panel.cwd())))
                .padding([2, 8])
                .max_width(160.0)
                .class(cursor_style(i == tabs.active_index(), pane_active));
            bar = bar.push(
                mouse_area(label)
                    .on_press(Message::SelectTab(side, i))
                    .on_middle_press(Message::CloseTabAt(side, i))
                    .on_double_click(Message::CloseTabAt(side, i)),
            );
        }
        bar.into()
    }
    ```
    Build the pane column as: an optional tab bar (`if tabs.items().len() > 1`), then path, header, list, status. Use `column![].push_maybe(...)`, or push conditionally.
  - Imports: `use crate::app::Tab;` and `use shagoff_core::tabs::Tabs;`.

- [ ] **Step 4: Verify**

  Run: `just verify`

  Expected: clean, with 46 core tests and 6 app tests. Then run `cargo run -p shagoff-commander` and take a screenshot with `grim` to check the tab bar layout. Don't send keystrokes with `wtype`, because they go to whichever window has focus.

- [ ] **Step 5: Commit**
  ```bash
  git add crates/app && git commit -m "app: tabs per pane with TC shortcuts and tab bar"
  ```

---

### Task 4: Docs and PR

**Files:**
- Modify: `TESTING.md` (manual checklist for phase 3)
- Modify: `.claude/docs/ROADMAP.md`:
  - tick phase 3;
  - tick the tech-debt item about modifiers (the Phase 4 Shift/Alt item).
- Modify: `.claude/docs/tc-reference.md`:
  - add the mouse behaviour for tabs (middle click and double click close a tab);
  - add a note on Ctrl+Tab if the compositor grabs it.
- Modify: `CLAUDE.md`: in "Implemented so far", add `tabs`.

- [ ] **Step 1: Add the checklist to TESTING.md**
  ```markdown
  ### Фаза 3: вкладки
  - [ ] Одна вкладка — полосы нет; Ctrl+T — появилась полоса, новая вкладка справа от текущей, тот же каталог и курсор
  - [ ] Ctrl+Tab / Ctrl+Shift+Tab переключают по кругу; клик по вкладке выбирает её и активирует панель
  - [ ] Ctrl+W закрывает текущую; на последней ничего не происходит; средняя кнопка и двойной клик по вкладке закрывают
  - [ ] Ctrl+T и Ctrl+W работают в русской раскладке
  - [ ] Вкладка с `/usr/lib`, прокрученная вниз, и вкладка с `~`: при переключении у каждой своя прокрутка и курсор
  - [ ] Во вкладках разных панелей каталоги независимы; Tab по-прежнему переключает панели
  - [ ] Shift+Tab и Alt+Enter ничего не делают
  ```

- [ ] **Step 2: Commit, push and open the PR**
  ```bash
  git add TESTING.md .claude CLAUDE.md && git commit -m "docs: phase 3 checklist, roadmap, tc-reference"
  git push -u origin feat/tabs
  gh pr create --title "Tabs (phase 3)" --body "Spec: .claude/docs/specs/2026-10-02-tabs.md
  Plan: .claude/docs/plans/03-tabs.md"
  gh pr checks --watch
  ```

- [ ] **Step 3: Merge only after the user approves.**
