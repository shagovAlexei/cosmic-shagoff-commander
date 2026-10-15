# 81. The drop menu at the pointer

ROADMAP «Drag & drop, дальше: меню броска у указателя (сейчас — окно)».

TC: files dragged with the right button and dropped open a small popup menu where the button was
released (Copy / Move / Link / Cancel), not a dialog.

- Where the files are held comes from the window's own DnD events (`route_event`: Offer `Enter` /
  `Motion`, window coordinates) as `Message::DropAt`; `App::drop_at` keeps the last one. Not a
  zone's `on_drop`: a zone inside a scrollable (rows, tabs, drive buttons) gets points shifted by
  the scroll offset.
- `Dialog::DropMenu` carries that point (`at`). `dialogs::view` draws it not as a dialog but as a
  window-sized layer: a libcosmic `popover` with `Position::Point(at)` (it keeps the menu inside
  the window, flipping it above the point near the bottom edge) and a menu card
  (`Container::Dropdown`, 240 px): «N files: dropped here», the target dir (cut in the middle),
  then `menu_button` rows Copy, Move, Link, Cancel. A press outside the card cancels, as Esc.
- Not yet: arrow keys / letters in the menu (TC has them); Esc and the mouse work.
- Picking an item works as before: `DropAs(op)` → the F5 / F6 / link dialog aimed at the dir.
