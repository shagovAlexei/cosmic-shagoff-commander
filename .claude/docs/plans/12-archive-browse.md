# 12. Archive as a folder (read-only) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Enter on zip / tar.* / 7z browses it like a dir; F5, Ctrl+C, Enter and F3 extract from it; everything that would change it says "read-only".

**Architecture:** A path through an archive (`/x/a.zip/docs`) is a plain `PathBuf`. `listing::scan` falls back to `archive::list` when `read_dir` fails and `archive::split_path` finds an archive, so `Panel`, history, tabs and the path line need no change. Extraction reuses the phase 11 unpack machinery (staging + `transfer(Move)`) with an entry filter.

**Tech Stack:** Rust 2024, crates from phase 11 (zip, tar, flate2, bzip2, liblzma, zstd, sevenz-rust2), libcosmic (pinned).

**Spec:** `.claude/docs/specs/12-archive-browse.md`

## Global Constraints

- `crates/core` has no libcosmic; logic there, test-first.
- Every new UI string in `fl!`, `en` and `ru`.
- Tests use tempdirs only; `App::build` never touches the real temp root (cleanup lives in `init`).
- Nothing changes an archive; nothing outside the chosen destination is written.
- `just verify` green; commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. Extract with a selection must still refuse `..`/absolute/symlink-parent entries inside the selected subtree — Task 2 `extract_selected_evil_entry_stays_inside`.
2. A path that merely does not exist (deleted dir) must not be mistaken for an archive path — Task 1 `split_path_*`.
3. The index cache must not serve a stale listing after the archive changes — Task 1 `list_sees_rewritten_archive`.
4. Read-only guard covers the target side too (F5 / Ctrl+V / Alt+F9 into an archive panel) — Task 4 app tests.
5. Session restore / `existing_dir` with a path inside a deleted archive falls back to a real dir — Task 1 `existing_dir_inside_archive`.

---

### Task 1: Index, `split_path`, `list`, scan and session fallback

**Files:** `crates/core/src/archive.rs`, `crates/core/src/listing.rs`, `crates/core/src/session.rs`

**Produces:** `Format::is_tree`, `archive::split_path(&Path) -> Option<(PathBuf, PathBuf)>`, `archive::list(&Path, &Path, bool) -> io::Result<Vec<Entry>>`, `listing::ext_of` made `pub(crate)`.

Tests (archive.rs): `split_path_real_dir_is_none`, `split_path_root_and_deep`, `split_path_missing_dir_is_none`, `split_path_non_archive_file_is_none`, `list_zip_tar_7z_with_implicit_dirs`, `list_hides_dot_files`, `list_missing_inner_is_not_found`, `list_sees_rewritten_archive`; listing.rs `scan_inside_archive`; session.rs `existing_dir_inside_archive`.

Implementation:
- `Item { path, dir, link, size, mtime, mode: Option<u32> }`, `fn read_index(f, archive) -> io::Result<Vec<Item>>` — zip via `by_index_raw` (no decompression), 7z via `sevenz_rust2::Archive::open(path).files`, tar.* via headers; paths through `safe_path`, unsafe ones dropped.
- Cache: `static INDEX: Mutex<Option<(PathBuf, u64, SystemTime, Arc<Vec<Item>>)>>`.
- `list`: children of `inner` by first component; explicit entries win over implicit dirs; implicit dir mtime = newest inside; `inner` absent → NotFound, a file → NotADirectory; default modes 0o644 / 0o755.
- `split_path`: real dir → None; walk `ancestors()` from `p`, stop at the first real dir; first regular file with `is_tree()` format wins.
- `listing::scan`: on `read_dir` error of kind NotADirectory/NotFound, `split_path` → `archive::list`; else the original error.
- `session::existing_dir`: an ancestor also qualifies when `split_path` recognises it.

Commit: `feat(core): list archives as dirs`.

### Task 2: `extract`

**Files:** `crates/core/src/archive.rs`

**Produces:** `archive::extract(archive, inner, names, dest, h) -> Report`.

Refactor `unpack` into a shared `run(units, dest, h)` where a unit = archive + optional own-dir name + optional selection `(inner, names)` + the paths to report as completed. `Stage` gets `select: Option<(PathBuf, Vec<PathBuf>)>`; `put` skips entries outside `inner/<name>` silently and strips `inner`; hardlink targets map the same way (unselected → refused).

Tests: `extract_one_file`, `extract_dir_subtree`, `extract_two_names`, `extract_conflict_skip`, `extract_cancel_leaves_no_staging`, `extract_selected_evil_entry_stays_inside`; all phase 11 unpack tests still pass.

Commit: `feat(core): extract chosen entries from an archive`.

### Task 3: Temp root

**Files:** `crates/core/src/archive.rs`

**Produces:** `archive::temp_root() -> PathBuf` (`$XDG_RUNTIME_DIR/shagoff-commander` or `temp_dir()/shagoff-commander-<uid>`), `archive::fresh_temp_dir(root) -> io::Result<PathBuf>` (`<root>/<pid>/<n>`, created), `archive::clean_temp(root)` (removes `<root>/<pid>` of dead pids, checked via `/proc/<pid>`).

Tests: `fresh_temp_dirs_differ`, `clean_temp_removes_dead_pids_only` (on a tempdir root).

Commit: `feat(core): temp dirs for files opened from archives`.

### Task 4: App

**Files:** `crates/app/src/{app.rs,jobs.rs,dialogs.rs,keymap.rs}`, i18n en/ru.

- keymap: Ctrl+PgDn → `Action::Enter` (test `ctrl_pgdn_enters`).
- jobs: `Job::Extract { archive, inner, names, dest }` → `archive::extract`.
- `OpKind::Extract` ("Extracting" / «Извлечение»); `Running.open: Option<Vec<OsString>>` run after a successful job.
- `act()`: `read_only(side, action)` guard before `dialog_for` → status `fl!("archive-read-only")`.
- Enter: archive file → `load(cwd/name)`; file inside an archive → extract to `fresh_temp_dir` + `xdg-open`. F3 inside an archive → same with the viewer.
- ClipCopy inside an archive → `Job::Extract` into the other pane's dir.
- `start_transfer` (F5): source dir inside an archive → `Job::Extract`; target inside an archive → read-only error.
- Unpack dialog target inside an archive → read-only error.
- `init`: `archive::clean_temp(&temp_root())`; `Message::Exit`: remove `<temp_root>/<pid>`.

Tests: `enter_on_archive_goes_inside_and_back`, `f8_inside_archive_is_read_only`, `f5_inside_archive_starts_extract`, `f5_into_archive_panel_is_read_only`, `ctrl_c_inside_archive_extracts`.

Commit: `feat(app): browse archives as folders`.

### Task 5: Docs + verify

tc-reference (Enter / Ctrl+PgDn on an archive, read-only note), ROADMAP (tick phase 12), TESTING.md section «Архив как папка (12)», CLAUDE.md (scan fallback, extract). `just verify`. Commit `docs: archive browsing (phase 12)`.
