# 88. Drag a tab to the other pane

ROADMAP «Вкладки, дальше: перетаскивание вкладки в другую панель».

TC: a tab dragged onto the other panel moves there.

- A tab press starts `App::tab_drag` (as for reordering). While it is held, a subscription reports
  the pointer's x (`Message::TabDragAt`); a pane's own mouse area would not hear the pointer come
  in, its rows take the moves. Main window events only (a popup has its own coordinates). Past
  `view::pane_split(skin, left list width)` — the middle of the gap, from the same constants the
  layout uses (Classic: gap 4; Modern: padding 8, card padding 6, gap 8) — it is the right pane
  (`App::pane_hover`). Only x counts: a release on the right half of the footer counts as the
  right pane (the panes' vertical bounds are not known to the app).
- The release (`ColDragEnd`, from any widget) over the other pane runs `Action::TabMoveOther`,
  with Ctrl `TabCopyOther` — the menu's code: the last tab cannot be moved (its message), it can
  be copied; the other pane becomes active.
- Over its own pane: nothing new (reordering on the tab bar as before). Not while the Ctrl+Q
  viewer covers the other pane. The release also ends a header-edge drag (cleanup first).
