# 30. Operations: Shift+F5, rename on conflict, dir dates, `~` Implementation Plan

**Goal:** Shift+F5 copies the cursor file in place; the conflict dialog can rename (one / all); copied dirs keep their mtime; `~` expands in the F5/F6 target.

**Spec:** `.claude/docs/specs/30-ops-more.md`

## Review Focus

1. A `Rename` name can never leave the destination dir (`valid_name`, also in `archive::place`).
2. A chosen name that is taken is asked about again, never silently replaced.
3. Move fast path with a rename that then crosses devices copies to the new name, not the old one.
4. Dir mtime is set after the children are written, only on dirs the copy created.

### Task 1: core
`Resolution::{Rename, RenameAll}`, `unique_name`, `valid_name`, `Transfer::destination`, dir mtime; `archive::place` loop. Commit `feat(core): rename on conflict, keep dir dates`.

### Task 2: app
`Action::CopySame` (Shift+F5, menu, help), `Dialog::Conflict.name` + field and buttons, `~` via `session::expand_home`. Commit `feat(app): Shift+F5, rename in the conflict dialog, ~ in targets`.

### Task 3: docs
tc-reference, ROADMAP, TESTING. Commit `docs: operations (phase 30)`.
