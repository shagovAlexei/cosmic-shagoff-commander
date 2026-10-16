# 97. Hotlist: drag and drop

ROADMAP «Избранное, дальше: перетаскивание».

- Ctrl+D → «Настроить…»: a row held with the left button follows the pointer (TC); the order is kept
  on OK, as the ↑ / ↓ buttons. Rows are buttons (they keep the press), so the press / release comes
  from an `event::listen_with` subscription while the dialog is open, the row under the pointer
  from a `mouse_area` per row (`HotMsg::Hover`, `Grab`, `Release`).
- Not done: dropping dirs from a panel onto the list — the dialog is modal (libcosmic's popover
  takes the input behind it), so a drag can't start in a panel while it is open.
- Robustness: a row's leave forgets it only if it is still the hovered one (enter / leave of
  neighbours come in either order); the wheel forgets it (another row is under the pointer); the
  scrollbar sits beside the rows; the slot just vacated is ignored once (rows of other heights
  shift under a still pointer).
