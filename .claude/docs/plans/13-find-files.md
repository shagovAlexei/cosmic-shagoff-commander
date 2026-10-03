# 13. Find files (Alt+F7) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Goal:** Alt+F7 dialog: name mask, start dir, optional text, background search with streamed results, Enter jumps to the file.

**Architecture:** `crates/core/src/search.rs` (pure walk + content match, stop flag, callbacks). App: `Dialog::Find(Box<Find>)`, a worker thread streaming `FindEvent`s over a futures channel tagged with a search id, like `jobs.rs`.

**Spec:** `.claude/docs/specs/13-find-files.md`

## Global Constraints
- core without libcosmic, test-first; `fl!` en+ru; tempdirs only; `just verify` green; commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus
1. Symlink loops (`a/loop -> ..`) must not hang — Task 1 `symlink_dir_loop_terminates`.
2. Text spanning a 256 KiB chunk boundary is found — Task 1 `text_across_chunk_boundary`.
3. Events of a previous search after a restart are ignored — Task 2 `stale_search_events_ignored`.
4. Stop / closing the dialog really stops the thread (stop flag checked per entry and per chunk) — Task 1 `stop_interrupts`.
5. A huge result set keeps the dialog responsive (render cap 5000) — Task 2 view code.

### Task 1: core `search.rs`
Tests: `mask_matches_nested`, `mask_exclusion`, `hidden_only_when_asked`, `text_case_insensitive_cyrillic`, `text_case_sensitive`, `text_across_chunk_boundary`, `symlink_dir_loop_terminates`, `stop_interrupts`, `dir_matches_without_text_only`, `skips_virtual_fs_roots`.
API: `Query { mask, text, case_sensitive, hidden }`, `find(root, q, stop, found, dir)`, `contains(path, q)`.
Commit `feat(core): find files by mask and text`.

### Task 2: app
keymap Alt+F7 → `Action::FindFiles` (test). `dialogs::Find { side, mask, dir, text, case_sensitive, results, total, current, cursor, id, stop }`, view with fields, checkbox, Find/Stop, list (cap 5000), count. `find.rs` worker: `spawn(id, root, Query) -> (Arc<AtomicBool>, Receiver<FindEvent>)`, events `Found(id, Vec<PathBuf>)` batched ≤100 ms, `Dir(id, PathBuf)`, `Done(id)`. Messages `FindInput(FindField, String)`, `FindCase`, `FindStart`, `FindStop`, `Find(FindEvent)`, `FindPick(usize)`. Keys while open: Up/Down move cursor, Enter → go if a result is selected and no field has text focus… simplified: Enter in fields = start search; Enter via key (FieldKey Enter not captured) on list → `FindPick(cursor)`; Escape stops + closes.
Tests: `alt_f7_opens_find_with_panel_dir`, `found_appends_results`, `stale_search_events_ignored`, `pick_goes_to_file`.
Commit `feat(app): Alt+F7 find files`.

### Task 3: docs + verify
tc-reference Alt+F7 → 13; ROADMAP tick + backlog line «В панель», фильтры; TESTING section «Поиск файлов (13)»; CLAUDE.md module list. `just verify`. Commit `docs: find files (phase 13)`.
