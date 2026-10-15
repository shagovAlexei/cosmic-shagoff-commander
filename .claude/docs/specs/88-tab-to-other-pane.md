# 88. Drag a tab to the other pane

ROADMAP «Вкладки, дальше: перетаскивание вкладки в другую панель».

TC: a tab dragged onto the other panel moves there.

- A tab press starts `App::tab_drag` (as for reordering). While it is held, a subscription reports
  the pointer's x (`Message::TabDragAt`); a pane's own mouse area would not hear the pointer come
  in, its rows take the moves. Past the left pane's list width (+ the gap) it is the right pane
  (`App::pane_hover`).
- The release (`ColDragEnd`, from any widget) over the other pane runs `Action::TabMoveOther`,
  with Ctrl `TabCopyOther` — the menu's code: the last tab cannot be moved (its message), it can
  be copied; the other pane becomes active.
- Over its own pane: nothing new (reordering on the tab bar as before).
