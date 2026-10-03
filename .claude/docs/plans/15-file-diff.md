# 15. File compare by content Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Goal:** Ctrl+Shift+D shows two files side by side with differences highlighted and next/prev navigation.
**Architecture:** `crates/core/src/diff.rs` (pure, `similar` line diff → aligned rows + block starts; binary fallback). App: `Dialog::Diff`, compare in `spawn_blocking`, monospace fixed-height rows, `scroll_to` the current block.
**Spec:** `.claude/docs/specs/15-file-diff.md`

## Global Constraints
core without libcosmic, test-first; `fl!` en+ru; tempdirs only; `just verify`; the dialog layout is checked by the user (letter shortcuts cannot be sent headless); commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus
1. Replace blocks pair up line by line; unequal sizes leave Deleted/Inserted tails — `changed_block_pairs`.
2. Missing trailing newline does not produce a phantom difference — `no_trailing_newline`.
3. Binary/large files never go through the text path (no huge allocations) — `binary_*`, `large_is_binary`.
4. Stale result (old id) ignored; ↓/↑ clamp at ends — app tests.
5. 10 000-row cap keeps the dialog responsive.

### Task 1: core `diff.rs` — tests `same_files`, `insert_delete_change`, `changed_block_pairs`, `blocks_mark_runs`, `empty_vs_text`, `no_trailing_newline`, `binary_same_and_differ`, `large_is_binary`. Commit `feat(core): line diff for file compare`.
### Task 2: app — keymap, dialog, messages `DiffReady(u64, Result<Outcome, String>)`, `DiffNext`, `DiffPrev`; tests `ctrl_shift_d_two_marked`, `ctrl_shift_d_cursor_pair`, `ctrl_shift_d_nothing`, `next_prev_clamp`, `stale_diff_ignored`. Commit `feat(app): compare files by content`.
### Task 3: docs — tc-reference, ROADMAP, TESTING, CLAUDE.md; verify. Commit `docs: file compare (phase 15)`.
