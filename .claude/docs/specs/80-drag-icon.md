# 80. A picture under the pointer while dragging

ROADMAP «Drag & drop, дальше: картинка под указателем».

- `dnd::drag_source` takes an `icon` closure; on drag start it goes to the compositor as the
  drag icon surface (`clipboard::start_dnd` with an `IconSurface`), 12 px right and below the
  pointer.
- `clip::drag_card`: a card (≤ 320 px, a long name cut with «…») with the theme icon and a
  label — one file: its icon and name; marked files (or one path that is not the dragged row,
  e.g. Ctrl+B with a dir and its file marked): `edit-copy` and «N items». Built when the drag
  starts, not per row and frame.
- The offset (12, 12) only holds on the Wayland subsurface path; libcosmic's buffer fallback
  (X11) puts the card's corner at the pointer.
- The headless sway check cannot finish a drag (the drop never arrives even on main), so the
  picture is checked by hand (TESTING § 80).
- Esc while one of our rows is dragged: the compositor does not cancel a drag on Esc, so the app
  does (`App::dragging`, set by `DragStart`, cleared by `DragEnd` — the source's end — and by Esc,
  which sends `iced::runtime::dnd::end_dnd()`).
