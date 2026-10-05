# 29. Brief view and type icons Implementation Plan

**Goal:** Ctrl+F1 / Ctrl+F2 switch the active tab between Brief (names in columns) and Full; every row gets a theme icon by MIME type.

**Spec:** `.claude/docs/specs/29-brief-icons.md`

## Global Constraints

- Logic (column math, icon names) in core with tests; the view only lays out.
- `mime_guess` is already in `Cargo.lock` (via rust-embed): add it to core without new downloads, `--locked` must still build.
- New strings (menu items, help) in en and ru. `just verify` green.

## Review Focus

1. Left/Right reach the panel only when no text field holds focus (command line caret must still move).
2. Brief column offset follows the cursor after every move, resize and dir change.
3. Old `PaneState` without `brief` still loads.

### Task 1: core
`viewport::{brief_rows, brief_cols, brief_first_col}`, `format::icon_name`, `PaneState.brief`. Commit `feat(core): brief column math, type icon names`.

### Task 2: app
`Action::{Left, Right, ViewBrief, ViewFull}`, `Tab.{brief, col, width}`, `Message::Resized` with width, `Message::BriefWheel`, brief view + icons, menu, help, i18n. Commit `feat(app): brief view, type icons`.

### Task 3: docs
tc-reference (Ctrl+F1/F2, ←/→), ROADMAP, TESTING.md. Commit `docs: brief view and icons (phase 29)`.
