# Clipboard Ctrl+C / Ctrl+X / Ctrl+V Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ctrl+C / Ctrl+X put the panel's targets on the system clipboard in cosmic-files' format; Ctrl+V pastes files from it (ours or cosmic-files') into the active panel.

**Architecture:** `shagoff-core::clipboard` encodes/decodes the three MIME payloads (pure, tested). A small `crates/app/src/clip.rs` adapts that to iced's `AsMimeTypes` / `AllowedMimeTypes` and returns `Task`s. Paste goes through the existing `App::start_transfer`, so progress, conflicts, errors and `SameFile` come for free.

**Tech Stack:** Rust 2024, libcosmic (rev `ef490df`), `cosmic::iced::clipboard::{write_data, read_data, write}`, crate `url` 2 (already in `Cargo.lock`).

**Spec:** `.claude/docs/specs/09-clipboard.md`

## Global Constraints

- Format = cosmic-files `src/clipboard.rs`: `x-special/gnome-copied-files` = `copy|cut` + `\n` + one `file://` URI per line; `text/uri-list` = each URI + `\r\n`; `text/plain` / `text/plain;charset=utf-8` / `UTF8_STRING` = paths joined by `\r\n`, non-UTF-8 paths skipped.
- Read accepts `x-special/gnome-copied-files` (preferred) and `text/uri-list` (kind `Copy`).
- Ctrl+V pastes immediately into the active pane's cwd; same dir → `plan-same-file` error as with F5.
- Cut: clipboard cleared when the move starts (only if it actually started).
- Ctrl+C/X/V inside a text field stay with the field — already true via `not_for_text`; do not change it.
- `crates/core` must not depend on libcosmic. `Cargo.lock` must not gain new packages (CI builds `--locked`).
- No new UI strings (nothing user-visible is added).

Deviation from spec: no app test for "`ClipCopy` without targets writes nothing" — the returned `Task` is opaque in tests; the branch is a plain `is_empty()` check, covered by the manual checklist.

## Review Focus

1. Pasting into the directory the files came from → status line shows `plan-same-file`, no job, clipboard NOT cleared (for cut). Test in Task 3.
2. Paste arriving while a job or dialog is open (read is async) → ignored. Test in Task 3.
3. Names with `%`, `#`, space, Cyrillic survive the round trip (percent-encoding). Test in Task 1.
4. A `text/uri-list` from another app with `#` comment lines and a trailing empty line → parsed, comments skipped. Test in Task 1.
5. Clipboard holding plain text or an `http://` URI → nothing happens. Test in Task 1 (`decode` → `None`) + Task 3 (`Pasted(None)`).

---

### Task 1: core `clipboard` module

**Files:**
- Create: `crates/core/src/clipboard.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod clipboard;` in alphabetical order, before `drives`)
- Modify: `crates/core/Cargo.toml` (add `url = "2"` to `[dependencies]`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Kind { Copy, Cut }
  #[derive(Clone, Debug, PartialEq, Eq)] pub struct Mime { pub gnome: String, pub uri_list: String, pub plain: String }
  pub const GNOME: &str = "x-special/gnome-copied-files";
  pub const URI_LIST: &str = "text/uri-list";
  pub fn encode(kind: Kind, paths: &[PathBuf]) -> Mime;
  pub fn decode(mime: &str, data: &[u8]) -> Option<(Kind, Vec<PathBuf>)>;
  ```

- [ ] **Step 1: Write the failing tests** — `crates/core/src/clipboard.rs` with module doc, stubs `todo!()` and tests:

```rust
//! System clipboard payloads for files, in cosmic-files' format
//! (pop-os/cosmic-files `src/clipboard.rs`, GPL-3.0-only).

use std::path::PathBuf;
use url::Url;

pub const GNOME: &str = "x-special/gnome-copied-files";
pub const URI_LIST: &str = "text/uri-list";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Cut,
}

/// The three payloads one copy/cut offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mime {
    /// `copy|cut` then one `file://` URI per line.
    pub gnome: String,
    /// Each URI followed by `\r\n`.
    pub uri_list: String,
    /// Paths joined by `\r\n`; non-UTF-8 paths are left out.
    pub plain: String,
}

pub fn encode(kind: Kind, paths: &[PathBuf]) -> Mime {
    todo!()
}

/// `None` for an unknown mime type, a non-`file://` URI or a malformed header.
pub fn decode(mime: &str, data: &[u8]) -> Option<(Kind, Vec<PathBuf>)> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<PathBuf> {
        vec!["/tmp/a b".into(), "/tmp/Отчёт 100%#1.txt".into()]
    }

    #[test]
    fn gnome_round_trip_keeps_kind_and_odd_names() {
        for kind in [Kind::Copy, Kind::Cut] {
            let m = encode(kind, &paths());
            assert_eq!(decode(GNOME, m.gnome.as_bytes()), Some((kind, paths())));
        }
    }

    #[test]
    fn gnome_layout_matches_cosmic_files() {
        let m = encode(Kind::Cut, &["/tmp/a b".into()]);
        assert_eq!(m.gnome, "cut\nfile:///tmp/a%20b");
        assert_eq!(m.uri_list, "file:///tmp/a%20b\r\n");
        assert_eq!(encode(Kind::Copy, &["/x".into()]).gnome, "copy\nfile:///x");
    }

    #[test]
    fn uri_list_round_trip_is_copy() {
        let m = encode(Kind::Cut, &paths());
        assert_eq!(decode(URI_LIST, m.uri_list.as_bytes()), Some((Kind::Copy, paths())));
    }

    #[test]
    fn plain_is_paths_joined_by_crlf() {
        assert_eq!(encode(Kind::Copy, &paths()).plain, "/tmp/a b\r\n/tmp/Отчёт 100%#1.txt");
    }

    #[test]
    fn uri_list_skips_comments_and_blank_lines() {
        let data = "# from some app\r\nfile:///tmp/x\r\n\r\n";
        assert_eq!(decode(URI_LIST, data.as_bytes()), Some((Kind::Copy, vec!["/tmp/x".into()])));
    }

    #[test]
    fn gnome_accepts_crlf_and_trailing_newline() {
        let data = "copy\r\nfile:///tmp/x\r\n";
        assert_eq!(decode(GNOME, data.as_bytes()), Some((Kind::Copy, vec!["/tmp/x".into()])));
    }

    #[test]
    fn rejects_non_file_uri_bad_header_and_unknown_mime() {
        assert_eq!(decode(URI_LIST, b"https://example.com/a\r\n"), None);
        assert_eq!(decode(GNOME, b"move\nfile:///tmp/x"), None);
        assert_eq!(decode(GNOME, b""), None);
        assert_eq!(decode("text/plain", b"/tmp/x"), None);
        assert_eq!(decode(URI_LIST, b"not a uri"), None);
    }
}
```

- [ ] **Step 2: Add deps and module, run tests to verify they fail**

Add `url = "2"` to `crates/core/Cargo.toml` `[dependencies]` (alphabetically after `trash`), and `pub mod clipboard;` to `lib.rs`.

Run: `cargo test -p shagoff-core clipboard`
Expected: compiles; 7 tests FAIL with `not yet implemented`. Also `git diff --stat Cargo.lock` shows only the `url` line added to shagoff-core's dependency list (no new `[[package]]`).

- [ ] **Step 3: Implement**

```rust
pub fn encode(kind: Kind, paths: &[PathBuf]) -> Mime {
    let mut m = Mime {
        gnome: match kind {
            Kind::Copy => "copy",
            Kind::Cut => "cut",
        }
        .into(),
        uri_list: String::new(),
        plain: String::new(),
    };
    for p in paths {
        if let Some(s) = p.to_str() {
            if !m.plain.is_empty() {
                m.plain.push_str("\r\n");
            }
            m.plain.push_str(s);
        }
        if let Ok(url) = Url::from_file_path(p) {
            m.uri_list.push_str(url.as_str());
            m.uri_list.push_str("\r\n");
            m.gnome.push('\n');
            m.gnome.push_str(url.as_str());
        }
    }
    m
}

pub fn decode(mime: &str, data: &[u8]) -> Option<(Kind, Vec<PathBuf>)> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let kind = match mime {
        GNOME => match lines.next()? {
            "copy" => Kind::Copy,
            "cut" => Kind::Cut,
            _ => return None,
        },
        URI_LIST => Kind::Copy,
        _ => return None,
    };
    let paths = lines
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| Url::parse(l).ok()?.to_file_path().ok())
        .collect::<Option<Vec<_>>>()?;
    Some((kind, paths))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shagoff-core clipboard`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/clipboard.rs crates/core/src/lib.rs crates/core/Cargo.toml Cargo.lock
git commit -m "core: clipboard payloads in cosmic-files format"
```

---

### Task 2: keymap Ctrl+C / Ctrl+X / Ctrl+V

**Files:**
- Modify: `crates/app/src/keymap.rs` (enum `Action`, physical-key table, tests)

**Interfaces:**
- Produces: `Action::ClipCopy`, `Action::ClipCut`, `Action::ClipPaste`.

- [ ] **Step 1: Write the failing test** (in `mod tests`, next to `ctrl_letters_use_physical_key`):

```rust
    #[test]
    fn ctrl_c_x_v_clipboard() {
        assert_eq!(chr("c", Code::KeyC, CTRL), Some(Action::ClipCopy));
        assert_eq!(chr("с", Code::KeyC, CTRL), Some(Action::ClipCopy)); // Russian layout
        assert_eq!(chr("x", Code::KeyX, CTRL), Some(Action::ClipCut));
        assert_eq!(chr("ч", Code::KeyX, CTRL), Some(Action::ClipCut));
        assert_eq!(chr("v", Code::KeyV, CTRL), Some(Action::ClipPaste));
        assert_eq!(chr("м", Code::KeyV, CTRL), Some(Action::ClipPaste));
        assert_eq!(chr("c", Code::KeyC, NONE), None);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shagoff-commander keymap`
Expected: compile error `no variant named ClipCopy`.

- [ ] **Step 3: Implement** — in `enum Action`, after `SwapPanes`:

```rust
    /// Ctrl+C / Ctrl+X: put the targets on the system clipboard; Ctrl+V: paste files from it.
    ClipCopy,
    ClipCut,
    ClipPaste,
```

In the physical-key `match`, after the `KeyU` row:

```rust
        (Physical::Code(Code::KeyC), true, false) => Some(Action::ClipCopy),
        (Physical::Code(Code::KeyX), true, false) => Some(Action::ClipCut),
        (Physical::Code(Code::KeyV), true, false) => Some(Action::ClipPaste),
```

In `app.rs` `act`, add a no-op arm so it compiles (replaced in Task 3), next to `Action::HistoryList | Action::Hotlist => {}`:

```rust
            Action::ClipCopy | Action::ClipCut | Action::ClipPaste => {}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p shagoff-commander keymap`
Expected: all keymap tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/keymap.rs crates/app/src/app.rs
git commit -m "keymap: Ctrl+C/X/V clipboard actions"
```

---

### Task 3: app — write, read, paste

**Files:**
- Create: `crates/app/src/clip.rs`
- Modify: `crates/app/src/main.rs` (`mod clip;` after `mod app;`)
- Modify: `crates/app/src/app.rs` (`Message::Pasted`, `act` arms, `update` arm, tests)
- Modify: `.claude/docs/tc-reference.md` (row Ctrl+C/X/V: `backlog` → done, mark like other done rows), `TESTING.md` (section 09)

**Interfaces:**
- Consumes: `shagoff_core::clipboard::{Kind, encode, decode, GNOME, URI_LIST}`, `Action::ClipCopy|ClipCut|ClipPaste`, `App::start_transfer(op: InputOp, side, sources: Vec<PathBuf>, input: &str)`.
- Produces: `Message::Pasted(Option<(Kind, Vec<PathBuf>)>)`; `clip::put<M>(Kind, &[PathBuf]) -> cosmic::app::Task<M>`, `clip::clear<M>() -> Task<M>`, `clip::take() -> Task<Message>`.

- [ ] **Step 1: Write the failing tests** (in `app.rs` `mod tests`; add `use shagoff_core::clipboard::Kind as ClipKind;` to the test imports):

```rust
    /// Point the active tab of `side` at `dir` and deliver its listing.
    fn listed_at(app: &mut App, side: usize, dir: &Path) {
        let _ = app.load(side, dir.into(), None);
        let t = app.panes[side].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: dir.into(),
            focus: None,
            result: Ok(listing::scan(dir, false).unwrap()),
            space: None,
        });
    }

    /// tmp/src/a (file) and tmp/dst/ (empty); pane 0 shows dst.
    fn paste_setup() -> (tempfile::TempDir, App, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a"), "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &dst);
        (tmp, app, src.join("a"))
    }

    #[test]
    fn paste_copy_starts_copy_into_active_cwd() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Copy));
    }

    #[test]
    fn paste_cut_starts_move() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    }

    #[test]
    fn paste_nothing_or_empty_does_nothing() {
        let (_tmp, mut app, _) = paste_setup();
        let _ = app.update(Message::Pasted(None));
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, Vec::new()))));
        assert!(app.job.is_none());
        assert!(app.panes[0].active().error.is_none());
    }

    #[test]
    fn paste_into_source_dir_reports_same_file() {
        let (_tmp, mut app, a) = paste_setup();
        listed_at(&mut app, 0, a.parent().unwrap());
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a.clone()]))));
        assert!(app.job.is_none());
        assert_eq!(
            app.panes[0].active().error.as_deref(),
            Some(fl!("plan-same-file", path = a.display().to_string()).as_str())
        );
    }

    #[test]
    fn paste_ignored_while_dialog_open() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Key(Action::Mkdir)); // opens the F7 dialog
        assert!(app.dialog.is_some());
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
        assert!(app.job.is_none());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p shagoff-commander paste`
Expected: compile error `no variant named Pasted`.

- [ ] **Step 3: Implement `clip.rs`**

```rust
//! System clipboard glue: `shagoff_core::clipboard` payloads ↔ iced mime traits.

use crate::app::Message;
use cosmic::app::Task;
use cosmic::iced::clipboard::{
    self,
    mime::{AllowedMimeTypes, AsMimeTypes},
};
use shagoff_core::clipboard::{self as fmt, Kind, Mime};
use std::borrow::Cow;
use std::path::PathBuf;

struct Files(Mime);

impl AsMimeTypes for Files {
    fn available(&self) -> Cow<'static, [String]> {
        Cow::Owned(
            [
                "text/plain",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                fmt::URI_LIST,
                fmt::GNOME,
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        let s = match mime_type {
            "text/plain" | "text/plain;charset=utf-8" | "UTF8_STRING" => &self.0.plain,
            fmt::URI_LIST => &self.0.uri_list,
            fmt::GNOME => &self.0.gnome,
            _ => return None,
        };
        Some(Cow::Owned(s.clone().into_bytes()))
    }
}

struct Paste(Kind, Vec<PathBuf>);

impl AllowedMimeTypes for Paste {
    fn allowed() -> Cow<'static, [String]> {
        Cow::Owned(vec![fmt::GNOME.into(), fmt::URI_LIST.into()])
    }
}

impl TryFrom<(Vec<u8>, String)> for Paste {
    type Error = ();
    fn try_from((data, mime): (Vec<u8>, String)) -> Result<Self, ()> {
        fmt::decode(&mime, &data).map(|(k, p)| Paste(k, p)).ok_or(())
    }
}

/// Ctrl+C / Ctrl+X.
pub fn put<M>(kind: Kind, paths: &[PathBuf]) -> Task<M> {
    clipboard::write_data(Files(fmt::encode(kind, paths)))
}

/// After a cut is pasted, so a second Ctrl+V does not try to move files that are gone.
pub fn clear<M>() -> Task<M> {
    clipboard::write(String::new())
}

/// Ctrl+V: files on the clipboard, if any, arrive as `Message::Pasted`.
pub fn take() -> Task<Message> {
    clipboard::read_data::<Paste>()
        .map(|p| cosmic::Action::App(Message::Pasted(p.map(|Paste(k, v)| (k, v)))))
}
```

If `write_data`/`write` return `Task<M>` where the libcosmic `Task<M>` alias expects `Task<cosmic::Action<M>>`, call them with the explicit type: `clipboard::write_data::<cosmic::Action<M>>(...)`.

- [ ] **Step 4: Wire it into `app.rs`**

Import: `use crate::clip;` and `use shagoff_core::clipboard::Kind as ClipKind;`.

`enum Message`, after `SearchInput(String)`:

```rust
    /// Files read from the system clipboard by Ctrl+V (`None`: no files there).
    Pasted(Option<(ClipKind, Vec<PathBuf>)>),
```

In `act`, replace the Task 2 placeholder arm with:

```rust
            Action::ClipCopy | Action::ClipCut => {
                let paths = panel.targets();
                if !paths.is_empty() {
                    let kind = if action == Action::ClipCut {
                        ClipKind::Cut
                    } else {
                        ClipKind::Copy
                    };
                    return clip::put(kind, &paths);
                }
            }
            Action::ClipPaste => return clip::take(),
```

In `update`, a new arm (next to `Message::Op`):

```rust
            Message::Pasted(Some((kind, paths)))
                if !paths.is_empty() && self.job.is_none() && self.dialog.is_none() =>
            {
                let op = match kind {
                    ClipKind::Copy => InputOp::Copy,
                    ClipKind::Cut => InputOp::Move,
                };
                // "": `cwd.join("")` ends with `/`, so `plan` always puts the files inside cwd.
                let task = self.start_transfer(op, self.active, paths, "");
                if kind == ClipKind::Cut && self.job.is_some() {
                    return Task::batch([clip::clear(), task]);
                }
                return task;
            }
            Message::Pasted(_) => {}
```

(Adapt `return` / trailing expression to the surrounding arm style of `update`.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p shagoff-commander paste`
Expected: 5 passed.

Run: `just verify`
Expected: fmt, clippy `-D warnings`, all tests green.

- [ ] **Step 6: Docs**

`tc-reference.md` row `Ctrl+C / Ctrl+X / Ctrl+V`: change status `backlog` to the done marker the other implemented rows use, last column `09`.

`TESTING.md`, new section after «История, избранное, обмен панелей (08)»:

```markdown
### Буфер обмена (09)
- [ ] Ctrl+C на файлах → Ctrl+V в cosmic-files вставляет копии; Ctrl+X → Ctrl+V в cosmic-files переносит
- [ ] Ctrl+C / Ctrl+X в cosmic-files → Ctrl+V у нас копирует / переносит в активную панель, с прогрессом
- [ ] Ctrl+V в каталог-источник → в строке состояния «нельзя скопировать файл сам в себя»; после вырезания буфер очищен (повторный Ctrl+V ничего не делает)
- [ ] Ctrl+C в поле быстрого поиска (Alt+буква) копирует текст, а не файлы; работает на русской раскладке
```

- [ ] **Step 7: Manual run**

`cargo run -p shagoff-commander` next to `cosmic-files`; walk through the TESTING.md section.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/clip.rs crates/app/src/main.rs crates/app/src/app.rs .claude/docs/tc-reference.md TESTING.md
git commit -m "app: Ctrl+C/X/V through the system clipboard"
```
