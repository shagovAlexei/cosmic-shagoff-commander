# Фаза 2: две панели — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A window with two TC-style panels: keyboard and mouse navigation, sorting, path line, status line, F-key bar.

**Architecture:**
- All logic that can be pure goes into `shagoff-core` and is unit-tested: formatting, totals, viewport math.
- `crates/app` holds:
  - `App` with two `Pane`s;
  - `keymap.rs`: key → `Action`, unit-tested;
  - `view.rs`: our own virtualized list, built from a `scrollable` with spacers.

**Tech Stack:** Rust 2024, libcosmic (pinned rev), `jiff` 0.2 (already in the tree), `tokio` (`spawn_blocking`).

**Spec:** `.claude/docs/specs/02-dual-pane.md`

## Global Constraints

- `crates/core` must not depend on libcosmic. The only new core dependency allowed is `jiff = "0.2"`.
- Every UI string goes through `fl!`, with both `en` and `ru` in `crates/app/i18n/*/shagoff-commander.ftl`.
- Row height is 22 px. Fallback list height is 400 px until the scrollable reports its real bounds.
- Date format `%d.%m.%Y %H:%M`, size `1 204 567`, folders `[name]` and `<DIR>`.
- Ctrl+R and Ctrl+\ match on the physical key (`Code::KeyR`, `Code::Backslash`).
- `just verify` must be clean. Branch: `feat/dual-pane`.

## Review Focus

1. **A scan result arrives for a path the user already left** (fast typing, or Backspace repeated). It must be dropped, so the panel shows only its latest location. The `pending` check is in Task 4, and its manual check is in TESTING.md.
2. **Huge directory (10 000+ files).** Only about 40 rows may be rendered, and the cursor must stay visible on PgDn and End. Tested by `visible_range_*` and `scroll_to_cursor_*` in Task 2, plus a manual check on `/usr/lib`.
3. **Unreadable directory (e.g. `/root`).** The error text goes into the status line, and the panel stays where it was. The `Err` branch is in Task 4, plus a manual check.
4. **Russian keyboard layout active.** Ctrl+R and Ctrl+\ must still work. Tested by `ctrl_letters_use_physical_key` in Task 3.
5. **Empty directory or `/`.** Status line shows `0 б`, Enter does nothing, no panic. Tested by `totals_skip_parent_row` and `visible_range_empty` in Task 2.

---

### Task 1: core — `Entry.mode` and `format`

**Files:**
- Modify: `crates/core/Cargo.toml` (`jiff = "0.2"`)
- Modify: `crates/core/src/listing.rs` (add a `mode` field and fill it in `scan`)
- Modify: `crates/core/src/panel.rs`, `crates/core/src/sort.rs` (add `mode: 0` to the `Entry` literals in `parent_entry` and the test helpers)
- Modify: `crates/core/src/lib.rs` (`pub mod format;`)
- Create: `crates/core/src/format.rs`

**Interfaces:**
- Produces:
  - `Entry.mode: u32`
  - `format::size(u64) -> String`
  - `format::date(SystemTime, &jiff::tz::TimeZone) -> String`
  - `format::perms(u32) -> String`
  - `format::display_name(&Entry) -> String`
  - `pub use jiff::tz::TimeZone` re-exported from `format`, so the app doesn't need its own `jiff` dependency

- [ ] **Step 1: Add the `mode` field (compiles, no behaviour yet)**
  - Add `pub mode: u32,` to `Entry`, after `is_link`, with the doc comment `/// Unix permission bits (of the link target for symlinks).`
  - In `scan`, add `mode: 0,` to the push for now.
  - Add `mode: 0,` to `parent_entry` in panel.rs, to test helper `f` in panel.rs, and to test helper `e` in sort.rs.

  Run: `cargo test -p shagoff-core`

  Expected: 24 passed.

- [ ] **Step 2: Write the failing tests**

  Add a test to `listing.rs` tests:
  ```rust
  #[test]
  fn scan_reads_mode() {
      use std::os::unix::fs::PermissionsExt;
      let d = tempfile::tempdir().unwrap();
      let p = d.path().join("x");
      fs::write(&p, "").unwrap();
      fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
      let v = scan(d.path(), true).unwrap();
      assert_eq!(v[0].mode & 0o777, 0o640);
  }
  ```

  Then add `pub mod format;` to `lib.rs`, add `jiff = "0.2"` under `[dependencies]` in `crates/core/Cargo.toml`, and create `crates/core/src/format.rs`:
  ```rust
  //! TC-style display formatting for the file table.

  use crate::listing::Entry;
  pub use jiff::tz::TimeZone;
  use std::time::SystemTime;

  pub fn size(_n: u64) -> String {
      todo!()
  }

  pub fn date(_t: SystemTime, _tz: &TimeZone) -> String {
      todo!()
  }

  pub fn perms(_mode: u32) -> String {
      todo!()
  }

  pub fn display_name(_e: &Entry) -> String {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::listing::Kind;
      use std::time::{Duration, UNIX_EPOCH};

      fn entry(name: &str, ext: &str, kind: Kind) -> Entry {
          Entry {
              name: name.into(),
              os_name: name.into(),
              ext: ext.into(),
              size: 0,
              mtime: UNIX_EPOCH,
              kind,
              is_link: false,
              mode: 0,
          }
      }

      #[test]
      fn size_groups_thousands_with_spaces() {
          assert_eq!(size(0), "0");
          assert_eq!(size(999), "999");
          assert_eq!(size(1000), "1 000");
          assert_eq!(size(1_204_567), "1 204 567");
          assert_eq!(size(u64::MAX), "18 446 744 073 709 551 615");
      }

      #[test]
      fn date_in_given_zone() {
          let t = UNIX_EPOCH + Duration::from_secs(1_790_948_940); // 2026-10-02 13:49 UTC
          assert_eq!(date(t, &TimeZone::UTC), "02.10.2026 13:49");
          let msk = TimeZone::fixed(jiff::tz::offset(3));
          assert_eq!(date(t, &msk), "02.10.2026 16:49");
      }

      #[test]
      fn perms_rwx() {
          assert_eq!(perms(0o755), "rwxr-xr-x");
          assert_eq!(perms(0o100640), "rw-r-----");
          assert_eq!(perms(0), "---------");
      }

      #[test]
      fn display_name_tc_style() {
          assert_eq!(display_name(&entry("src", "", Kind::Dir)), "[src]");
          assert_eq!(display_name(&entry("..", "", Kind::Dir)), "[..]");
          assert_eq!(display_name(&entry("Cargo.toml", "toml", Kind::File)), "Cargo");
          assert_eq!(display_name(&entry("a.tar.gz", "gz", Kind::File)), "a.tar");
          assert_eq!(display_name(&entry(".bashrc", "", Kind::File)), ".bashrc");
          assert_eq!(display_name(&entry("noext", "", Kind::File)), "noext");
      }
  }
  ```

- [ ] **Step 3: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core`

  Expected: 5 failures — 4 in `format` (`not yet implemented`) and `scan_reads_mode` (assertion: `0 != 0o640`).

- [ ] **Step 4: Implement**

  In `scan`, replace `mode: 0,` with `mode: meta.permissions().mode(),` and add `use std::os::unix::fs::PermissionsExt;` at the top of listing.rs. (`meta` is the target metadata, or the link's own metadata for a broken link.)

  In `format.rs`, replace the stubs with:
  ```rust
  /// `1204567` → `1 204 567`.
  pub fn size(n: u64) -> String {
      let digits = n.to_string();
      let mut out = String::with_capacity(digits.len() + digits.len() / 3);
      for (i, c) in digits.chars().enumerate() {
          if i > 0 && (digits.len() - i) % 3 == 0 {
              out.push(' ');
          }
          out.push(c);
      }
      out
  }

  /// `02.10.2026 13:49` in `tz`; empty if the time is out of range.
  pub fn date(t: SystemTime, tz: &TimeZone) -> String {
      jiff::Timestamp::try_from(t)
          .map(|ts| ts.to_zoned(tz.clone()).strftime("%d.%m.%Y %H:%M").to_string())
          .unwrap_or_default()
  }

  /// Permission bits as `rwxr-xr-x` (file type bits ignored).
  pub fn perms(mode: u32) -> String {
      (0..9)
          .map(|i| {
              let bit = 0o400 >> i;
              if mode & bit == 0 { '-' } else { ['r', 'w', 'x'][i % 3] }
          })
          .collect()
  }

  /// TC name column: dirs as `[name]`, files without the extension shown in its own column.
  pub fn display_name(e: &Entry) -> String {
      if e.is_dir() {
          format!("[{}]", e.name)
      } else if e.ext.is_empty() {
          e.name.clone()
      } else {
          e.name[..e.name.len() - e.ext.len() - 1].to_string()
      }
  }
  ```

- [ ] **Step 5: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core`

  Expected: 29 passed.

- [ ] **Step 6: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core Cargo.lock && git commit -m "core: permission bits and TC display formatting"
  ```

---

### Task 2: core — `set_cursor`, `totals`, `viewport`

**Files:**
- Modify: `crates/core/src/panel.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod viewport;`)
- Create: `crates/core/src/viewport.rs`

**Interfaces:**
- Consumes: `Panel` and `PARENT` (phase 1).
- Produces:
  - `Panel::set_cursor(&mut self, i: usize)`
  - `pub struct Totals { pub bytes: u64, pub files: usize, pub dirs: usize }` (derives `Debug, PartialEq, Eq, Default`) in panel.rs
  - `Panel::totals(&self) -> Totals`
  - `viewport::{visible_range(len: usize, row_h: f32, offset: f32, height: f32) -> Range<usize>, scroll_to_cursor(cursor: usize, row_h: f32, offset: f32, height: f32) -> Option<f32>, page_rows(row_h: f32, height: f32) -> usize}`

- [ ] **Step 1: Write the failing tests**

  In panel.rs:
  - add stubs `pub fn set_cursor(&mut self, _i: usize) { todo!() }` and `pub fn totals(&self) -> Totals { todo!() }`;
  - add the `Totals` struct above `impl Panel`:
    ```rust
    /// Status-line totals, excluding the `..` row.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Totals {
        pub bytes: u64,
        pub files: usize,
        pub dirs: usize,
    }
    ```
  - add these tests:
    ```rust
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
        assert_eq!(p.totals(), Totals { bytes: 1024, files: 2, dirs: 1 });
        assert_eq!(loaded("/", vec![]).totals(), Totals::default());
    }
    ```

  Add `pub mod viewport;` to `lib.rs` and create `crates/core/src/viewport.rs`:
  ```rust
  //! Scroll math for the virtualized file list (fixed row height).

  use std::ops::Range;

  /// Extra rows rendered above and below the viewport.
  pub const OVERSCAN: usize = 2;

  pub fn visible_range(_len: usize, _row_h: f32, _offset: f32, _height: f32) -> Range<usize> {
      todo!()
  }

  pub fn scroll_to_cursor(_cursor: usize, _row_h: f32, _offset: f32, _height: f32) -> Option<f32> {
      todo!()
  }

  pub fn page_rows(_row_h: f32, _height: f32) -> usize {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn visible_range_empty() {
          assert_eq!(visible_range(0, 20.0, 0.0, 400.0), 0..0);
      }

      #[test]
      fn visible_range_top_and_middle() {
          // 400 px / 20 px = 20 rows (+1 partial) + overscan
          assert_eq!(visible_range(10_000, 20.0, 0.0, 400.0), 0..23);
          // offset 1000 px → first visible row 50
          assert_eq!(visible_range(10_000, 20.0, 1000.0, 400.0), 48..73);
      }

      #[test]
      fn visible_range_clamped_to_len() {
          assert_eq!(visible_range(5, 20.0, 0.0, 400.0), 0..5);
          assert_eq!(visible_range(30, 20.0, 9999.0, 400.0), 28..30);
      }

      #[test]
      fn scroll_to_cursor_only_when_needed() {
          // rows 0..20 visible at offset 0
          assert_eq!(scroll_to_cursor(5, 20.0, 0.0, 400.0), None);
          // row 20 is just below → scroll so its bottom touches the viewport bottom
          assert_eq!(scroll_to_cursor(20, 20.0, 0.0, 400.0), Some(20.0));
          // row 3 is above the offset → scroll to its top
          assert_eq!(scroll_to_cursor(3, 20.0, 200.0, 400.0), Some(60.0));
          // cursor 0 after a dir change with an old offset
          assert_eq!(scroll_to_cursor(0, 20.0, 500.0, 400.0), Some(0.0));
      }

      #[test]
      fn page_rows_at_least_one() {
          assert_eq!(page_rows(20.0, 400.0), 19);
          assert_eq!(page_rows(20.0, 10.0), 1);
          assert_eq!(page_rows(20.0, 0.0), 1);
      }
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core`

  Expected: 7 failures (2 in panel, 5 in viewport), all `not yet implemented`.

- [ ] **Step 3: Implement**

  In panel.rs:
  ```rust
      pub fn set_cursor(&mut self, i: usize) {
          self.cursor = i;
          self.move_cursor(0); // clamp
      }

      pub fn totals(&self) -> Totals {
          let start = usize::from(self.parent_row());
          self.entries[start..].iter().fold(Totals::default(), |mut t, e| {
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

  In viewport.rs:
  ```rust
  /// Rows to render for a viewport at `offset` px with `height` px, plus `OVERSCAN` on each side.
  pub fn visible_range(len: usize, row_h: f32, offset: f32, height: f32) -> Range<usize> {
      if len == 0 || row_h <= 0.0 {
          return 0..0;
      }
      let first = ((offset.max(0.0) / row_h) as usize).min(len);
      let count = (height.max(0.0) / row_h).ceil() as usize + 1;
      first.saturating_sub(OVERSCAN)..(first + count + OVERSCAN).min(len)
  }

  /// Offset that brings row `cursor` fully into view, or `None` if it already is.
  pub fn scroll_to_cursor(cursor: usize, row_h: f32, offset: f32, height: f32) -> Option<f32> {
      let top = cursor as f32 * row_h;
      let bottom = top + row_h;
      if top < offset {
          Some(top)
      } else if bottom > offset + height {
          Some((bottom - height).max(0.0))
      } else {
          None
      }
  }

  /// PgUp/PgDn step: full rows on screen minus one, at least 1.
  pub fn page_rows(row_h: f32, height: f32) -> usize {
      ((height.max(0.0) / row_h) as usize).saturating_sub(1).max(1)
  }
  ```

  Check `visible_range(30, 20.0, 9999.0, 400.0)`: `first = min(499, 30) = 30`, so the range is `28..min(30 + 21 + 2, 30) = 28..30`. That matches the test.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core`

  Expected: 36 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core && git commit -m "core: set_cursor, status totals, viewport math"
  ```

---

### Task 3: app — `keymap`

**Files:**
- Create: `crates/app/src/keymap.rs`
- Modify: `crates/app/src/main.rs` (`mod keymap;`)

**Interfaces:**
- Consumes: `shagoff_core::sort::SortKey`.
- Produces:
  - `pub enum Action { SwitchPane, Up, Down, PageUp, PageDown, Home, End, Enter, Parent, Root, Reload, Sort(SortKey) }` (derives `Clone, Copy, Debug, PartialEq, Eq`)
  - `pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action>`, with `Key`, `Physical` and `Modifiers` from `cosmic::iced::keyboard`

- [ ] **Step 1: Write the failing tests**

  Add `mod keymap;` to main.rs and create `crates/app/src/keymap.rs`:
  ```rust
  //! TC key bindings → `Action`. One table; the F-key buttons dispatch the same actions.

  use cosmic::iced::keyboard::{Key, Modifiers, key::Physical};
  use shagoff_core::sort::SortKey;

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Action {
      SwitchPane,
      Up,
      Down,
      PageUp,
      PageDown,
      Home,
      End,
      Enter,
      Parent,
      Root,
      Reload,
      Sort(SortKey),
  }

  pub fn action(_key: &Key, _physical: Physical, _mods: Modifiers) -> Option<Action> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use cosmic::iced::keyboard::key::{Code, Named, NativeCode};

      const NONE: Modifiers = Modifiers::empty();
      const CTRL: Modifiers = Modifiers::CTRL;

      fn named(n: Named, mods: Modifiers) -> Option<Action> {
          action(&Key::Named(n), Physical::Unidentified(NativeCode::Unidentified), mods)
      }
      fn chr(c: &str, code: Code, mods: Modifiers) -> Option<Action> {
          action(&Key::Character(c.into()), Physical::Code(code), mods)
      }

      #[test]
      fn navigation_keys() {
          assert_eq!(named(Named::Tab, NONE), Some(Action::SwitchPane));
          assert_eq!(named(Named::ArrowUp, NONE), Some(Action::Up));
          assert_eq!(named(Named::ArrowDown, NONE), Some(Action::Down));
          assert_eq!(named(Named::PageUp, NONE), Some(Action::PageUp));
          assert_eq!(named(Named::PageDown, NONE), Some(Action::PageDown));
          assert_eq!(named(Named::Home, NONE), Some(Action::Home));
          assert_eq!(named(Named::End, NONE), Some(Action::End));
          assert_eq!(named(Named::Enter, NONE), Some(Action::Enter));
          assert_eq!(named(Named::Backspace, NONE), Some(Action::Parent));
          assert_eq!(named(Named::PageUp, CTRL), Some(Action::Parent));
      }

      #[test]
      fn ctrl_f_keys_sort() {
          assert_eq!(named(Named::F3, CTRL), Some(Action::Sort(SortKey::Name)));
          assert_eq!(named(Named::F4, CTRL), Some(Action::Sort(SortKey::Ext)));
          assert_eq!(named(Named::F5, CTRL), Some(Action::Sort(SortKey::Date)));
          assert_eq!(named(Named::F6, CTRL), Some(Action::Sort(SortKey::Size)));
          assert_eq!(named(Named::F3, NONE), None); // F3 = view, phase 6
      }

      #[test]
      fn ctrl_letters_use_physical_key() {
          assert_eq!(chr("r", Code::KeyR, CTRL), Some(Action::Reload));
          assert_eq!(chr("к", Code::KeyR, CTRL), Some(Action::Reload)); // Russian layout
          assert_eq!(chr("\\", Code::Backslash, CTRL), Some(Action::Root));
          assert_eq!(chr("ё", Code::Backslash, CTRL), Some(Action::Root));
          assert_eq!(chr("r", Code::KeyR, NONE), None);
      }

      #[test]
      fn unbound_combinations() {
          assert_eq!(named(Named::ArrowUp, CTRL), None);
          assert_eq!(named(Named::Escape, NONE), None);
      }
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 4 failures, `not yet implemented`.

- [ ] **Step 3: Implement**
  ```rust
  pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
      use cosmic::iced::keyboard::key::{Code, Named};
      let ctrl = mods.control();
      if let Key::Named(n) = key {
          return Some(match (n, ctrl) {
              (Named::Tab, false) => Action::SwitchPane,
              (Named::ArrowUp, false) => Action::Up,
              (Named::ArrowDown, false) => Action::Down,
              (Named::PageUp, false) => Action::PageUp,
              (Named::PageDown, false) => Action::PageDown,
              (Named::Home, false) => Action::Home,
              (Named::End, false) => Action::End,
              (Named::Enter, false) => Action::Enter,
              (Named::Backspace, false) | (Named::PageUp, true) => Action::Parent,
              (Named::F3, true) => Action::Sort(SortKey::Name),
              (Named::F4, true) => Action::Sort(SortKey::Ext),
              (Named::F5, true) => Action::Sort(SortKey::Date),
              (Named::F6, true) => Action::Sort(SortKey::Size),
              _ => return None,
          });
      }
      // Letter shortcuts by physical key so they work in any layout.
      match (physical, ctrl) {
          (Physical::Code(Code::KeyR), true) => Some(Action::Reload),
          (Physical::Code(Code::Backslash), true) => Some(Action::Root),
          _ => None,
      }
  }
  ```

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 4 passed. `keymap` is not used by `main` yet, so expect a dead-code warning. Allow it for this task with `#![allow(dead_code)]` at the top of keymap.rs, and remove it in Task 4.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy --all-targets -- -D warnings
  git add crates/app && git commit -m "app: TC keymap with layout-independent Ctrl shortcuts"
  ```

---

### Task 4: app — two panes: state, update, view

**Files:**
- Modify: `crates/app/Cargo.toml` (`tokio = { version = "1", features = ["rt"] }`)
- Modify: `crates/app/src/main.rs` (flags from argv, `mod view;`, window size)
- Modify: `crates/app/src/app.rs` (state, update, subscription)
- Create: `crates/app/src/view.rs`
- Modify: `crates/app/i18n/en/shagoff-commander.ftl`, `crates/app/i18n/ru/shagoff-commander.ftl`
- Remove: `#![allow(dead_code)]` from keymap.rs

**Interfaces:**
- Consumes: everything from Tasks 1–3.
- Produces: `app::{App, Flags, Pane, Message, ROW_H}`. `view::view(&App)` and `view::fkey_bar()` are used by `App::view`.

This task is UI glue with no unit tests (the logic is tested in Tasks 1–3). It is verified by `just verify` and the manual checklist.

- [ ] **Step 1: Add the i18n strings**

  Append to `en/shagoff-commander.ftl`:
  ```
  col-name = Name
  col-ext = Ext
  col-size = Size
  col-date = Date
  col-attr = Attr
  status = { $sel_bytes } b of { $bytes } b, files { $sel_files } of { $files }, dirs { $sel_dirs } of { $dirs }
  fkey-view = F3 View
  fkey-edit = F4 Edit
  fkey-copy = F5 Copy
  fkey-move = F6 Move
  fkey-mkdir = F7 NewFolder
  fkey-delete = F8 Delete
  fkey-exit = Alt+F4 Exit
  open-failed = Cannot open: { $err }
  ```

  Append to `ru/shagoff-commander.ftl`:
  ```
  col-name = Имя
  col-ext = Расш.
  col-size = Размер
  col-date = Дата
  col-attr = Атрибуты
  status = { $sel_bytes } б из { $bytes } б, файлов { $sel_files } из { $files }, папок { $sel_dirs } из { $dirs }
  fkey-view = F3 Просмотр
  fkey-edit = F4 Правка
  fkey-copy = F5 Копия
  fkey-move = F6 Перемещ.
  fkey-mkdir = F7 Каталог
  fkey-delete = F8 Удалить
  fkey-exit = Alt+F4 Выход
  open-failed = Не удалось открыть: { $err }
  ```

- [ ] **Step 2: main.rs — flags and window size**
  ```rust
  mod app;
  mod i18n;
  mod keymap;
  mod view;

  fn main() -> cosmic::iced::Result {
      simple_logger::SimpleLogger::new()
          .with_level(log::LevelFilter::Warn)
          .env()
          .init()
          .ok();
      i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
      let flags = app::Flags {
          left: std::env::args_os().nth(1).map(Into::into),
      };
      let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(1200.0, 800.0));
      cosmic::app::run::<app::App>(settings, flags)
  }
  ```

- [ ] **Step 3: app.rs — state, update, subscription**

  Replace the whole of app.rs:
  ```rust
  use crate::fl;
  use crate::keymap::{self, Action};
  use cosmic::app::{Core, Task};
  use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
  use cosmic::iced::{Subscription, event, keyboard};
  use cosmic::{Application, Element, widget};
  use shagoff_core::format::TimeZone;
  use shagoff_core::listing::{self, Entry};
  use shagoff_core::panel::Panel;
  use shagoff_core::sort::SortKey;
  use shagoff_core::viewport;
  use std::path::PathBuf;

  pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";
  /// Fixed row height of the file list; the viewport math depends on it.
  pub const ROW_H: f32 = 22.0;
  // ponytail: list height is guessed until the scrollable reports its bounds (it does on the first event).
  const FALLBACK_LIST_H: f32 = 400.0;

  pub struct Flags {
      pub left: Option<PathBuf>,
  }

  pub struct Pane {
      pub panel: Panel,
      pub scroll_id: widget::Id,
      pub offset: f32,
      pub height: f32,
      /// Path of the scan in flight; results for any other path are stale.
      pending: Option<PathBuf>,
      pub error: Option<String>,
  }

  impl Pane {
      fn new(cwd: PathBuf) -> Self {
          Self {
              panel: Panel::new(cwd),
              scroll_id: widget::Id::unique(),
              offset: 0.0,
              height: FALLBACK_LIST_H,
              pending: None,
              error: None,
          }
      }
  }

  pub struct App {
      core: Core,
      pub panes: [Pane; 2],
      pub active: usize,
      pub tz: TimeZone,
  }

  #[derive(Debug, Clone)]
  pub enum Message {
      Key(Action),
      Listed {
          side: usize,
          path: PathBuf,
          focus: Option<String>,
          result: Result<Vec<Entry>, String>,
      },
      Click(usize, usize),
      DoubleClick(usize, usize),
      Header(usize, SortKey),
      /// side, scroll offset y, viewport height
      Scrolled(usize, f32, f32),
      Exit,
  }

  impl Application for App {
      type Executor = cosmic::executor::Default;
      type Flags = Flags;
      type Message = Message;
      const APP_ID: &'static str = APP_ID;

      fn core(&self) -> &Core {
          &self.core
      }

      fn core_mut(&mut self) -> &mut Core {
          &mut self.core
      }

      fn init(mut core: Core, flags: Flags) -> (Self, Task<Message>) {
          core.window.header_title = fl!("app-title");
          let home = std::env::home_dir().unwrap_or_else(|| "/".into());
          let left = flags
              .left
              .and_then(|p| p.canonicalize().ok())
              .filter(|p| p.is_dir())
              .unwrap_or_else(|| home.clone());
          let mut app = Self {
              core,
              panes: [Pane::new(left.clone()), Pane::new(home.clone())],
              active: 0,
              tz: TimeZone::system(),
          };
          let task = Task::batch([app.load(0, left, None), app.load(1, home, None)]);
          (app, task)
      }

      fn update(&mut self, message: Message) -> Task<Message> {
          match message {
              Message::Key(action) => return self.act(self.active, action),
              Message::Listed { side, path, focus, result } => {
                  let pane = &mut self.panes[side];
                  if pane.pending.as_ref() != Some(&path) {
                      return Task::none(); // stale: the user has moved on
                  }
                  pane.pending = None;
                  match result {
                      Ok(entries) => {
                          pane.error = None;
                          pane.panel.set_listing(path, entries, focus.as_deref());
                          return self.reveal(side);
                      }
                      Err(e) => pane.error = Some(e),
                  }
              }
              Message::Click(side, i) => {
                  self.active = side;
                  self.panes[side].panel.set_cursor(i);
              }
              Message::DoubleClick(side, i) => {
                  self.active = side;
                  self.panes[side].panel.set_cursor(i);
                  return self.act(side, Action::Enter);
              }
              Message::Header(side, key) => {
                  self.active = side;
                  return self.act(side, Action::Sort(key));
              }
              Message::Scrolled(side, offset, height) => {
                  self.panes[side].offset = offset;
                  self.panes[side].height = height;
              }
              Message::Exit => return cosmic::iced::exit(),
          }
          Task::none()
      }

      fn subscription(&self) -> Subscription<Message> {
          event::listen_with(|event, status, _| match event {
              cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                  key,
                  physical_key,
                  modifiers,
                  ..
              }) if status == event::Status::Ignored => {
                  keymap::action(&key, physical_key, modifiers).map(Message::Key)
              }
              _ => None,
          })
      }

      fn view(&self) -> Element<'_, Message> {
          crate::view::view(self)
      }

      fn footer(&self) -> Option<Element<'_, Message>> {
          Some(crate::view::fkey_bar())
      }
  }

  impl App {
      /// Scan `path` in the background; the result lands in `Message::Listed`.
      fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
          let pane = &mut self.panes[side];
          pane.pending = Some(path.clone());
          let show_hidden = pane.panel.show_hidden();
          Task::perform(
              async move {
                  let p = path.clone();
                  let result = tokio::task::spawn_blocking(move || listing::scan(&p, show_hidden))
                      .await
                      .map_err(|e| e.to_string())
                      .and_then(|r| r.map_err(|e| e.to_string()));
                  Message::Listed { side, path, focus, result }
              },
              cosmic::Action::App,
          )
      }

      fn act(&mut self, side: usize, action: Action) -> Task<Message> {
          let page = viewport::page_rows(ROW_H, self.panes[side].height) as isize;
          let panel = &mut self.panes[side].panel;
          match action {
              Action::SwitchPane => {
                  self.active = 1 - self.active;
                  return Task::none();
              }
              Action::Up => panel.move_cursor(-1),
              Action::Down => panel.move_cursor(1),
              Action::PageUp => panel.move_cursor(-page),
              Action::PageDown => panel.move_cursor(page),
              Action::Home => panel.cursor_home(),
              Action::End => panel.cursor_end(),
              Action::Sort(key) => panel.set_sort(key),
              Action::Enter => {
                  if let Some((path, focus)) = panel.enter_path() {
                      return self.load(side, path, focus);
                  }
                  if let Some(e) = panel.current() {
                      let file = panel.cwd().join(&e.os_name);
                      if let Err(err) = open_detached(&file) {
                          self.panes[side].error = Some(fl!("open-failed", err = err.to_string()));
                      }
                  }
              }
              Action::Parent => {
                  if let Some((path, focus)) = panel.parent_path() {
                      return self.load(side, path, Some(focus));
                  }
              }
              Action::Root => return self.load(side, "/".into(), None),
              Action::Reload => {
                  let cwd = panel.cwd().to_path_buf();
                  return self.load(side, cwd, None);
              }
          }
          self.reveal(side)
      }

      /// Scroll so the cursor row is fully visible.
      fn reveal(&mut self, side: usize) -> Task<Message> {
          let pane = &mut self.panes[side];
          match viewport::scroll_to_cursor(pane.panel.cursor(), ROW_H, pane.offset, pane.height) {
              Some(y) => {
                  pane.offset = y;
                  scrollable::scroll_to(pane.scroll_id.clone(), AbsoluteOffset { x: None, y: Some(y) })
              }
              None => Task::none(),
          }
      }
  }

  /// `xdg-open` without blocking the UI or leaving a zombie.
  fn open_detached(path: &std::path::Path) -> std::io::Result<()> {
      let mut child = std::process::Command::new("xdg-open").arg(path).spawn()?;
      std::thread::spawn(move || child.wait());
      Ok(())
  }
  ```

  If the borrow checker rejects `self.load`/`self.panes[side].error` while `panel` is borrowed, compute the needed values first, end the borrow, then call. That is a mechanical fix; ledger it as a ruling only if behaviour changes.

- [ ] **Step 4: view.rs**

  Create `crates/app/src/view.rs`:
  ```rust
  //! TC layout: per pane a path line, column headers, a virtualized file list and a status line.

  use crate::app::{App, Message, ROW_H};
  use crate::fl;
  use cosmic::iced::widget::{column, row};
  use cosmic::iced::{Alignment, Color, Length};
  use cosmic::widget::{self, button, container, mouse_area, scrollable, text};
  use cosmic::{Element, theme};
  use shagoff_core::format;
  use shagoff_core::listing::Entry;
  use shagoff_core::sort::SortKey;
  use shagoff_core::viewport;

  const W_EXT: f32 = 60.0;
  const W_SIZE: f32 = 110.0;
  const W_DATE: f32 = 130.0;
  const W_ATTR: f32 = 90.0;
  const TEXT: u16 = 13;

  pub fn view(app: &App) -> Element<'_, Message> {
      row![pane(app, 0), pane(app, 1)]
          .spacing(4)
          .height(Length::Fill)
          .into()
  }

  fn pane(app: &App, side: usize) -> Element<'_, Message> {
      let p = &app.panes[side];
      let active = app.active == side;
      let entries = p.panel.entries();
      let cursor = p.panel.cursor();

      let path = container(text(p.panel.cwd().display().to_string()).size(TEXT))
          .padding([2, 6])
          .width(Length::Fill)
          .class(bar_style(active));

      let range = viewport::visible_range(entries.len(), ROW_H, p.offset, p.height);
      let mut list = column![widget::Space::with_height(range.start as f32 * ROW_H)];
      for i in range.clone() {
          list = list.push(file_row(app, side, i, &entries[i], i == cursor, active));
      }
      list = list.push(widget::Space::with_height(
          (entries.len() - range.end) as f32 * ROW_H,
      ));
      let list = scrollable(list)
          .id(p.scroll_id.clone())
          .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
          .height(Length::Fill);

      let status: Element<_> = match &p.error {
          Some(e) => text(e.clone())
              .size(TEXT)
              .class(theme::Text::Custom(|t| cosmic::iced::widget::text::Style {
                  color: Some(t.cosmic().destructive_color().into()),
              }))
              .into(),
          None => {
              let t = p.panel.totals();
              text(fl!(
                  "status",
                  sel_bytes = "0",
                  bytes = format::size(t.bytes),
                  sel_files = "0",
                  files = t.files.to_string(),
                  sel_dirs = "0",
                  dirs = t.dirs.to_string()
              ))
              .size(TEXT)
              .into()
          }
      };

      column![
          path,
          header(side, p.panel.sort()),
          list,
          container(status).padding([2, 6])
      ]
      .width(Length::Fill)
      .into()
  }

  fn header(side: usize, sort: shagoff_core::sort::Sort) -> Element<'static, Message> {
      let cell = |label: String, key: SortKey, width: Length| {
          let arrow = match (sort.key == key, sort.asc) {
              (true, true) => " ▲",
              (true, false) => " ▼",
              _ => "",
          };
          mouse_area(container(text(format!("{label}{arrow}")).size(TEXT)).width(width).padding([2, 6]))
              .on_press(Message::Header(side, key))
      };
      row![
          cell(fl!("col-name"), SortKey::Name, Length::Fill),
          cell(fl!("col-ext"), SortKey::Ext, Length::Fixed(W_EXT)),
          cell(fl!("col-size"), SortKey::Size, Length::Fixed(W_SIZE)),
          cell(fl!("col-date"), SortKey::Date, Length::Fixed(W_DATE)),
          container(text(fl!("col-attr")).size(TEXT)).width(Length::Fixed(W_ATTR)).padding([2, 6]),
      ]
      .into()
  }

  fn file_row<'a>(
      app: &'a App,
      side: usize,
      i: usize,
      e: &'a Entry,
      is_cursor: bool,
      active: bool,
  ) -> Element<'a, Message> {
      let size = if e.is_dir() { "<DIR>".to_string() } else { format::size(e.size) };
      let cells = row![
          text(format::display_name(e)).size(TEXT).width(Length::Fill),
          text(e.ext.clone()).size(TEXT).width(Length::Fixed(W_EXT)),
          text(size).size(TEXT).width(Length::Fixed(W_SIZE)).align_x(Alignment::End),
          text(format::date(e.mtime, &app.tz)).size(TEXT).width(Length::Fixed(W_DATE)),
          text(format::perms(e.mode)).size(TEXT).width(Length::Fixed(W_ATTR)),
      ]
      .spacing(6)
      .align_y(Alignment::Center);
      let row = container(cells)
          .padding([0, 6])
          .height(Length::Fixed(ROW_H))
          .width(Length::Fill)
          .clip(true)
          .class(cursor_style(is_cursor, active));
      mouse_area(row)
          .on_press(Message::Click(side, i))
          .on_double_click(Message::DoubleClick(side, i))
          .into()
  }

  pub fn fkey_bar() -> Element<'static, Message> {
      let disabled = |label: String| button::standard(label).width(Length::Fill);
      row![
          disabled(fl!("fkey-view")),
          disabled(fl!("fkey-edit")),
          disabled(fl!("fkey-copy")),
          disabled(fl!("fkey-move")),
          disabled(fl!("fkey-mkdir")),
          disabled(fl!("fkey-delete")),
          button::standard(fl!("fkey-exit"))
              .on_press(Message::Exit)
              .width(Length::Fill),
      ]
      .spacing(4)
      .padding(4)
      .into()
  }

  fn bar_style(active: bool) -> theme::Container<'static> {
      theme::Container::custom(move |t| {
          let c = t.cosmic();
          if active {
              container::Style {
                  background: Some(Color::from(c.accent_color()).into()),
                  text_color: Some(c.on_accent_color().into()),
                  ..Default::default()
              }
          } else {
              container::Style::default()
          }
      })
  }

  fn cursor_style(is_cursor: bool, active: bool) -> theme::Container<'static> {
      theme::Container::custom(move |t| {
          if !is_cursor {
              return container::Style::default();
          }
          let c = t.cosmic();
          let mut bg = Color::from(c.accent_color());
          if active {
              container::Style {
                  background: Some(bg.into()),
                  text_color: Some(c.on_accent_color().into()),
                  ..Default::default()
              }
          } else {
              bg.a = 0.35;
              container::Style {
                  background: Some(bg.into()),
                  ..Default::default()
              }
          }
      })
  }
  ```

  Widget method names (`with_height`, `align_x`, `clip`, `Text::Custom`) may differ slightly in this libcosmic rev. Fix compile errors against `~/.cargo/git/checkouts/libcosmic-*/ef490df` sources. A rename that keeps the behaviour needs no ruling.

- [ ] **Step 5: Verify**

  Run: `just verify`

  Expected: fmt, clippy and tests are clean, 36 core + 4 app tests.

  Then run `cargo run -p shagoff-commander` and walk through the manual checklist (Task 5, Step 1).

- [ ] **Step 6: Commit**
  ```bash
  git add crates/app Cargo.lock && git commit -m "app: two TC panels with keyboard/mouse navigation"
  ```

---

### Task 5: Docs and PR

**Files:**
- Modify: `TESTING.md` (manual checklist for phase 2)
- Modify: `.claude/docs/ROADMAP.md`:
  - tick phase 2;
  - tick the tech-debt item "нормализовать cwd";
  - tick the item "Тест: каталоги сверху…" only if it was done, which this plan does not do, so leave it.
- Modify: `CLAUDE.md`: in "Implemented so far", list `format` and `viewport` in core, and `app.rs`, `keymap.rs`, `view.rs` in the app.

- [ ] **Step 1: Add the manual checklist to TESTING.md**

  Under "Ручной чек-лист UI":
  ```markdown
  ### Фаза 2: две панели
  - [ ] Старт: обе панели в `~`; `cargo run -p shagoff-commander -- /usr` открывает левую в `/usr`; несуществующий путь → `~`
  - [ ] Tab переключает панель, подсветка строки пути и курсора меняется
  - [ ] ↑↓ PgUp PgDn Home End; курсор не уходит за экран (проверить на `/usr/lib`)
  - [ ] Enter на папке входит, на `..` выходит и ставит курсор на папку, из которой вышли; Backspace и Ctrl+PgUp — то же
  - [ ] Enter на файле открывает его в программе по умолчанию
  - [ ] Ctrl+\ → `/`, Ctrl+R перечитывает; оба работают в русской раскладке
  - [ ] Ctrl+F3..F6 и клик по заголовку сортируют, повтор меняет ▲/▼, папки всегда сверху
  - [ ] Клик ставит курсор и активирует панель, двойной клик = Enter, колесо прокручивает
  - [ ] `/root` (нет прав): ошибка в строке состояния красным, панель осталась на месте
  - [ ] Быстрые повторные Backspace: панель оказывается в последнем каталоге, без «прыжков»
  - [ ] Светлая и тёмная тема: курсор и строка пути читаемы
  - [ ] «Alt+F4 Выход» закрывает окно, остальные F-кнопки неактивны
  ```

- [ ] **Step 2: Commit, push and open the PR**
  ```bash
  git add TESTING.md .claude CLAUDE.md && git commit -m "docs: phase 2 checklist, roadmap, CLAUDE.md"
  git push -u origin feat/dual-pane
  gh pr create --title "Two TC panels (phase 2)" --body "Spec: .claude/docs/specs/02-dual-pane.md
  Plan: .claude/docs/plans/02-dual-pane.md"
  gh pr checks --watch
  ```

- [ ] **Step 3: Merge only after the user approves.**
