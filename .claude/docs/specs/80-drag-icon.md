# 80. A picture under the pointer while dragging

ROADMAP «Drag & drop, дальше: картинка под указателем».

- `dnd::drag_source` takes an `icon` closure; on drag start it goes to the compositor as the
  drag icon surface (`clipboard::start_dnd` with an `IconSurface`), 12 px right and below the
  pointer.
- `clip::drag_icon`: a card with the theme icon and a label — one file: its icon and name;
  marked files: `edit-copy` and «N items».
- The headless sway check cannot finish a drag (the drop never arrives even on main), so the
  picture is checked by hand (TESTING § 80).
