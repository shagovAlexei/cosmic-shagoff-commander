# 103. Pane splitter — plan

Spec: `../specs/103-pane-splitter.md`.

1. Core: `viewport::{split_ratio, split_or_half, visible_cols, NAME_MIN}` (tests).
2. App: `State.split`, `App.{split, splitting}`, `Message::Split{Start, At, End, Reset}`, a mouse subscription while held (test).
3. View: panes by `FillPortion`, the divider `mouse_area`, footer halves by the share, columns by `visible_cols`, drive bar one line.
