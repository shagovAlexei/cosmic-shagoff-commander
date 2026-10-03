# 14. Compare and synchronize dirs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Goal:** Shift+F2 marks differing files in both panels; Ctrl+Shift+S compares the two panel dirs (recursively) and copies by per-row arrows.

**Architecture:** pure `crates/core/src/sync.rs` (`compare`, `default_dir`, `plan`, `compare_lists`) + `Panel::mark_names`. App: `Dialog::Sync`, compare in `spawn_blocking` tagged by id, `Job::Sync` = two `ops::transfer(Copy)` with a handler that answers conflicts with Replace.

**Spec:** `.claude/docs/specs/14-dir-sync.md`

## Global Constraints
core without libcosmic, test-first; `fl!` en+ru; tempdirs only; `just verify`; commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus
1. mtime within 2 s is "same", not "newer" — Task 1 `two_second_tolerance`.
2. File vs dir with the same name never gets an arrow — Task 1 `file_vs_dir_differs`.
3. A one-sided dir is one row and copies whole; both-sided dirs are descended — Task 1 `one_sided_dir_is_one_row`.
4. Sync replaces only by arrows; nothing deleted — Task 2 job test via `plan`.
5. Stale compare results (old id) ignored; closing stops the compare — Task 2.

### Task 1: core `sync.rs` + `Panel::mark_names`
Tests: `left_and_right_only`, `newer_by_mtime`, `two_second_tolerance`, `same_date_other_size_differs`, `file_vs_dir_differs`, `content_mode`, `ignore_date_mode`, `one_sided_dir_is_one_row`, `recursive_off_lists_top_only`, `hidden_skipped`, `plan_follows_arrows`, `compare_lists_marks_unique_and_newer`, panel `mark_names_replaces_marks`.
Commit `feat(core): compare and plan dir sync`.

### Task 2: app
keymap tests; `Dialog::Sync(Box<SyncDlg>)` with options, rows, id, stop, running; messages `SyncOpt(SyncOpt)`, `SyncCompare`, `SyncCompared(u64, Vec<Row>)`, `SyncFlip(usize)`, `SyncRun`; `Job::Sync`; `OpKind::Sync`.
Tests: `shift_f2_marks_both_panels`, `ctrl_shift_s_opens_sync`, `stale_compare_ignored`, `flip_cycles_direction`, `sync_run_starts_job`, `sync_in_archive_is_read_only`.
Commit `feat(app): compare and synchronize dirs`.

### Task 3: docs + verify
tc-reference, ROADMAP, TESTING «Сравнение и синхронизация (14)», CLAUDE.md. Commit `docs: dir sync (phase 14)`.
