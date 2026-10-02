# Фаза 4: выделение — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TC marking. Insert, Space, Shift+arrows, Num+/−/*, Ctrl+A, Ctrl+Num± and Ctrl+click mark files. Num+ and Num− open a mask dialog. Marked rows are shown in red, and the status line counts the marked entries.

**Architecture:**
- Core gets `Mask` (a TC glob) and marking on `Panel`. The marked set is keyed by `os_name`. All of it is unit-tested.
- The app gets new keymap actions, a modal mask dialog (`Application::dialog`), Ctrl tracking for clicks, and red text for marked rows.

**Tech Stack:** Rust 2024, libcosmic (pinned rev). No new crates.

**Spec:** `.claude/docs/specs/04-selection.md`

## Global Constraints

- `crates/core` has no UI dependencies and no new crates.
- `..` is never marked. Num+, Num− and Num* act on files only. Ctrl+A acts on files and dirs.
- Numpad keys match the physical code (`NumpadAdd`, `NumpadSubtract`, `NumpadMultiply`). Ctrl+A matches `KeyA`.
- New UI strings go into both `en` and `ru`.
- `just verify` must be clean. Branch: `feat/selection`.

## Review Focus

1. **Rescan while marks exist (Ctrl+R, or a file deleted outside).** Marks on files that still exist are kept, and marks on vanished files are dropped. Tested by `rescan_prunes_vanished_marks` in Task 2.
2. **Mask dialog open while the user presses panel keys** (arrows, Insert, Tab). Panels must not react, Escape closes the dialog, and typed characters go to the input. Checked by the dialog gate in Task 4 and manually.
3. **Wrong `+` key.** `+` on the main keyboard (Shift+=) must not open the dialog. Tested by `main_keyboard_plus_is_not_numpad` in Task 3.
4. **Marked row under the cursor.** It stays red and readable on the accent background in both themes. Manual check.
5. **`*.*` and names without a dot (Makefile).** They match, as in TC. Tested by `star_dot_star_matches_names_without_dot` in Task 1.

---

### Task 1: core — `Mask`

**Files:**
- Create: `crates/core/src/mask.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod mask;`)

**Interfaces:**
- Produces: `pub struct Mask` with `Mask::parse(&str) -> Mask` and `Mask::matches(&self, name: &str) -> bool`.

- [ ] **Step 1: Write the failing tests**

  Create `crates/core/src/mask.rs`:
  ```rust
  //! TC file mask: `*.rs;*.toml|*.bak` — patterns split by `;`, exclusions after `|`, `*` and `?`, case-insensitive.

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Mask {
      include: Vec<String>,
      exclude: Vec<String>,
  }

  impl Mask {
      pub fn parse(_s: &str) -> Self {
          todo!()
      }

      pub fn matches(&self, _name: &str) -> bool {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      fn m(mask: &str, name: &str) -> bool {
          Mask::parse(mask).matches(name)
      }

      #[test]
      fn star_and_question() {
          assert!(m("*.rs", "main.rs"));
          assert!(!m("*.rs", "main.rsx"));
          assert!(m("a?c", "abc"));
          assert!(!m("a?c", "ac"));
          assert!(m("*", "anything"));
          assert!(m("a*b*c", "aXXbYYc"));
          assert!(!m("a*b*c", "aXXbYY"));
      }

      #[test]
      fn case_insensitive() {
          assert!(m("*.RS", "Main.rs"));
          assert!(m("readme*", "README.md"));
      }

      #[test]
      fn several_patterns_and_exclusions() {
          assert!(m("*.rs;*.toml", "Cargo.toml"));
          assert!(!m("*.rs;*.toml", "Cargo.lock"));
          assert!(m("*|*.bak", "a.txt"));
          assert!(!m("*|*.bak", "a.bak"));
          assert!(!m("*.rs|test*", "test_main.rs"));
      }

      #[test]
      fn star_dot_star_matches_names_without_dot() {
          assert!(m("*.*", "Makefile"));
          assert!(m("*.*", "a.b"));
      }

      #[test]
      fn empty_mask_means_all() {
          assert!(m("", "x"));
          assert!(m("  ", "x"));
          assert!(m("|*.bak", "x.txt"));
      }
  }
  ```
  Add `pub mod mask;` to `lib.rs`.

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib mask`

  Expected: 5 failures, `not yet implemented`.

- [ ] **Step 3: Implement**
  ```rust
  impl Mask {
      pub fn parse(s: &str) -> Self {
          let (inc, exc) = s.split_once('|').unwrap_or((s, ""));
          let split = |part: &str| -> Vec<String> {
              part.split(';')
                  .map(|p| p.trim().to_lowercase())
                  .filter(|p| !p.is_empty())
                  // TC: `*.*` means every file, including names without a dot
                  .map(|p| if p == "*.*" { "*".to_string() } else { p })
                  .collect()
          };
          let mut include = split(inc);
          if include.is_empty() {
              include.push("*".into());
          }
          Self { include, exclude: split(exc) }
      }

      pub fn matches(&self, name: &str) -> bool {
          let name = name.to_lowercase();
          self.include.iter().any(|p| glob(p, &name)) && !self.exclude.iter().any(|p| glob(p, &name))
      }
  }

  /// `*` = any run, `?` = one char. Greedy with backtracking to the last `*`.
  fn glob(pattern: &str, name: &str) -> bool {
      let p: Vec<char> = pattern.chars().collect();
      let n: Vec<char> = name.chars().collect();
      let (mut pi, mut ni) = (0, 0);
      let mut star: Option<(usize, usize)> = None;
      while ni < n.len() {
          if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
              pi += 1;
              ni += 1;
          } else if pi < p.len() && p[pi] == '*' {
              star = Some((pi, ni));
              pi += 1;
          } else if let Some((sp, sn)) = star {
              pi = sp + 1;
              ni = sn + 1;
              star = Some((sp, sn + 1));
          } else {
              return false;
          }
      }
      p[pi..].iter().all(|&c| c == '*')
  }
  ```

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib mask`

  Expected: 5 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core && git commit -m "core: TC file mask"
  ```

---

### Task 2: core — marks on `Panel`

**Files:**
- Modify: `crates/core/src/panel.rs`

**Interfaces:**
- Consumes: `Mask` (Task 1).
- Produces these `Panel` methods:
  - `toggle_mark(&mut self)`
  - `toggle_mark_and_move(&mut self, delta: isize)`
  - `mark_by_mask(&mut self, mask: &Mask, on: bool)`
  - `invert(&mut self)`
  - `mark_all(&mut self, on: bool)`
  - `is_marked(&self, e: &Entry) -> bool`
  - `marked_totals(&self) -> Totals`

- [ ] **Step 1: Write the failing tests**

  Add the field `marked: HashSet<OsString>` to `Panel`, initialised to `HashSet::new()` in `new`, and add `use std::collections::HashSet; use std::ffi::OsString; use crate::mask::Mask;`. Add stubs (`todo!()`) for the seven methods. Then add these tests:
  ```rust
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
      assert_eq!(p.marked_totals(), Totals { bytes: 1000, files: 1, dirs: 1 });
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib panel`

  Expected: 8 new tests fail with `not yet implemented`. The existing ones pass.

- [ ] **Step 3: Implement**

  In `set_listing`, update `marked` before `self.entries = entries;`:
  ```rust
          if same_dir {
              self.marked.retain(|k| entries.iter().any(|e| &e.os_name == k));
          } else {
              self.marked.clear();
          }
  ```

  Methods:
  ```rust
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
          for e in self.entries.iter().filter(|e| !e.is_dir() && mask.matches(&e.name)) {
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
              self.marked.extend(self.entries[start..].iter().map(|e| e.os_name.clone()));
          }
      }

      pub fn marked_totals(&self) -> Totals {
          sum(self.entries.iter().filter(|e| self.is_marked(e)))
      }
  ```

  Refactor `totals()` to use a shared free function:
  ```rust
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
  ```
  `totals()` becomes `sum(self.entries[start..].iter())`. `..` is never in `marked`, so `marked_totals` needs no skip.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib`

  Expected: 46 + 5 + 8 = 59 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core && git commit -m "core: TC marks on Panel (toggle, mask, invert, all, totals)"
  ```

---

### Task 3: keymap — marking keys

**Files:**
- Modify: `crates/app/src/keymap.rs`
- Modify: `crates/app/src/app.rs` (temporary no-op arms)

**Interfaces:**
- Produces: new `Action` variants `Mark`, `MarkDown`, `MarkUp`, `SelectGroup`, `UnselectGroup`, `Invert`, `SelectAll`, `UnselectAll`, `Cancel`.

- [ ] **Step 1: Write the failing tests**

  Add the variants, plus a temporary arm `Action::Mark | ... | Action::Cancel => {}` in `App::act` so the build stays green. Then add tests:
  ```rust
  #[test]
  fn marking_keys() {
      const SHIFT: Modifiers = Modifiers::SHIFT;
      assert_eq!(named(Named::Insert, NONE), Some(Action::MarkDown));
      assert_eq!(named(Named::Space, NONE), Some(Action::Mark));
      assert_eq!(named(Named::ArrowDown, SHIFT), Some(Action::MarkDown));
      assert_eq!(named(Named::ArrowUp, SHIFT), Some(Action::MarkUp));
      assert_eq!(named(Named::Escape, NONE), Some(Action::Cancel));
  }

  #[test]
  fn numpad_marking() {
      assert_eq!(chr("+", Code::NumpadAdd, NONE), Some(Action::SelectGroup));
      assert_eq!(chr("-", Code::NumpadSubtract, NONE), Some(Action::UnselectGroup));
      assert_eq!(chr("*", Code::NumpadMultiply, NONE), Some(Action::Invert));
      assert_eq!(chr("+", Code::NumpadAdd, CTRL), Some(Action::SelectAll));
      assert_eq!(chr("-", Code::NumpadSubtract, CTRL), Some(Action::UnselectAll));
      assert_eq!(chr("a", Code::KeyA, CTRL), Some(Action::SelectAll));
      assert_eq!(chr("ф", Code::KeyA, CTRL), Some(Action::SelectAll)); // Russian layout
  }

  #[test]
  fn main_keyboard_plus_is_not_numpad() {
      assert_eq!(chr("+", Code::Equal, Modifiers::SHIFT), None);
      assert_eq!(chr("-", Code::Minus, NONE), None);
  }
  ```
  The phase 2 test `unbound_combinations` asserts that Escape gives `None`. Change that line to `named(Named::F12, NONE)` and record the change as a ruling (Escape is now bound, by the spec).

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: `marking_keys` and `numpad_marking` fail. `main_keyboard_plus_is_not_numpad` already passes, because those keys are unbound, and that is correct.

- [ ] **Step 3: Implement**

  Add to the named-key match:
  ```rust
              (Named::Insert, false, false) | (Named::ArrowDown, false, true) => Action::MarkDown,
              (Named::ArrowUp, false, true) => Action::MarkUp,
              (Named::Space, false, false) => Action::Mark,
              (Named::Escape, false, false) => Action::Cancel,
  ```

  Add to the physical match:
  ```rust
          (Physical::Code(Code::NumpadAdd), false, false) => Some(Action::SelectGroup),
          (Physical::Code(Code::NumpadSubtract), false, false) => Some(Action::UnselectGroup),
          (Physical::Code(Code::NumpadMultiply), false, false) => Some(Action::Invert),
          (Physical::Code(Code::NumpadAdd | Code::KeyA), true, false) => Some(Action::SelectAll),
          (Physical::Code(Code::NumpadSubtract), true, false) => Some(Action::UnselectAll),
  ```

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 9 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy --all-targets -- -D warnings
  git add crates/app && git commit -m "app: TC marking keys (Insert, Space, Shift+arrows, numpad, Ctrl+A)"
  ```

---

### Task 4: app — wire marking, mask dialog, red rows, status

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/view.rs`
- Modify: `crates/app/i18n/en/shagoff-commander.ftl`, `crates/app/i18n/ru/shagoff-commander.ftl`

**Interfaces:**
- Consumes: Tasks 2 and 3.
- Produces: `Message::{Modifiers(Modifiers), MaskInput(String), MaskSubmit, MaskCancel}`, `App.mask_dialog: Option<MaskDialog>` and `App::dialog()`.

UI glue. It is verified by `just verify`, a screenshot and the manual checklist.

- [ ] **Step 1: i18n**

  en:
  ```
  select-group = Select group
  unselect-group = Unselect group
  ok = OK
  cancel = Cancel
  ```

  ru:
  ```
  select-group = Выделить группу
  unselect-group = Снять выделение
  ok = OK
  cancel = Отмена
  ```

- [ ] **Step 2: app.rs**
  - Add `pub struct MaskDialog { side: usize, select: bool, input: String }`.
  - Add the `App` fields `mods: Modifiers`, `mask_dialog: Option<MaskDialog>` and `mask_input_id: widget::Id`.
  - Subscription: also map `keyboard::Event::ModifiersChanged(m)` to `Message::Modifiers(m)`, whatever the status.
  - `update`:
    - `Message::Modifiers(m) => self.mods = m`.
    - `Message::Key(action)`: if `self.mask_dialog.is_some()`, handle only `Action::Cancel` (set `mask_dialog = None`) and ignore everything else.
    - `Message::Click(side, i)`: as before, then `if self.mods.control() { panel.toggle_mark() }`.
    - `MaskInput(s)`: update `input`.
    - `MaskSubmit`: take the dialog, then `self.panes[d.side].active_mut().panel.mark_by_mask(&Mask::parse(&d.input), d.select)`.
    - `MaskCancel`: set `mask_dialog = None`.
  - `act`:
    - `Mark` → `toggle_mark()`;
    - `MarkDown` → `toggle_mark_and_move(1)`;
    - `MarkUp` → `toggle_mark_and_move(-1)`;
    - `Invert` → `invert()`;
    - `SelectAll` → `mark_all(true)`;
    - `UnselectAll` → `mark_all(false)`;
    - `Cancel` → no-op;
    - `SelectGroup` / `UnselectGroup` → set `mask_dialog = Some(MaskDialog { side, select, input: "*".into() })` and return `cosmic::widget::text_input::focus(self.mask_input_id.clone())`.
    - The arms that move the cursor fall through to `reveal`, as before.
  - `fn dialog(&self) -> Option<Element<'_, Message>>`: build `widget::dialog()` with:
    - `.title(fl!(if select "select-group" else "unselect-group"))`;
    - `.control(widget::text_input("", &d.input).id(self.mask_input_id.clone()).on_input(Message::MaskInput).on_submit(|_| Message::MaskSubmit))`;
    - `.primary_action(widget::button::suggested(fl!("ok")).on_press(Message::MaskSubmit))`;
    - `.secondary_action(widget::button::standard(fl!("cancel")).on_press(Message::MaskCancel))`.

- [ ] **Step 3: view.rs**
  - `file_row` gets `is_marked = p.panel.is_marked(e)` and passes it to `cursor_style(is_cursor, active, is_marked)`.
  - In `cursor_style`, when `marked` is true, set `text_color = destructive_color` in every branch, including the plain no-cursor row, which now returns a style with only `text_color`.
  - `cursor_style` is also used by the tab bar: pass `false` for `marked` there.
  - Status: use `p.panel.marked_totals()` for `sel_bytes` (via `format::size`), `sel_files` and `sel_dirs`.

- [ ] **Step 4: Verify**

  Run: `just verify`

  Expected: clean, 59 core + 9 app tests.

  For the visual check, make an uncommitted temporary patch to `init` that marks a few entries after loading. Or just take a screenshot of the unmarked UI, and leave red rows and the dialog to the manual checklist. Do not send keystrokes.

- [ ] **Step 5: Commit**
  ```bash
  git add crates/app && git commit -m "app: marking, mask dialog, red marked rows, marked totals"
  ```

---

### Task 5: Docs and PR

- Modify `TESTING.md` — add this checklist:
  ```markdown
  ### Фаза 4: выделение
  - [ ] Insert отмечает и идёт вниз; Space отмечает на месте; Shift+↓/↑ отмечают и двигают; `[..]` не отмечается
  - [ ] Отмеченные строки красные, в том числе под курсором; читаемо в светлой и тёмной теме
  - [ ] Строка состояния: отмеченные байты/файлы/папки из общего числа
  - [ ] Num+ → диалог «Выделить группу», `*.rs` выделяет только файлы `.rs`, папки не трогает; Num− снимает по маске
  - [ ] В открытом диалоге стрелки/Insert/Tab не трогают панели, Escape закрывает, Enter применяет
  - [ ] Num* инвертирует файлы; Ctrl+A и Ctrl+Num+ выделяют всё, включая папки; Ctrl+Num− снимает всё
  - [ ] `+` на основной клавиатуре не открывает диалог; Ctrl+A работает в русской раскладке
  - [ ] Ctrl+клик переключает отметку; отметки переживают сортировку и Ctrl+R; при смене каталога сбрасываются
  ```
- ROADMAP: tick phase 4.
- CLAUDE.md: add `mask` to "Implemented so far", and state that marks exist.
- tc-reference: add Ctrl+click.
- Commit, push, then `gh pr create --title "TC selection (phase 4)"`, with a body that links the spec and the plan. Wait for CI. Merge only after the user approves.
