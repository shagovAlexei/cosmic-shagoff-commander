# 97. Hotlist: drag and drop

ROADMAP «Избранное, дальше: перетаскивание».

- Ctrl+D → «Настроить…»: a row held with the left button follows the pointer (TC); the order is kept
  on OK, as the ↑ / ↓ buttons. Rows are buttons (they keep the press), so the press / release comes
  from an `event::listen_with` subscription while the dialog is open, the row under the pointer
  from a `mouse_area` per row (`HotMsg::Hover`, `Grab`, `Release`).
- Dirs dragged from a panel onto that list are added after the selection (files left out).
