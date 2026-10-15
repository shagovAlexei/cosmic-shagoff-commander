# 81. The drop menu at the pointer

ROADMAP «Drag & drop, дальше: меню броска у указателя (сейчас — окно)».

TC: files dragged with the right button and dropped open a small popup menu where the button was
released (Copy / Move / Link / Cancel), not a dialog.

- Every `clip::drop_zone` also reports where the drop landed (`DndDestination::on_drop(x, y)`,
  window coordinates) as `Message::DropAt`; `App::drop_at` keeps it until the drop's data comes.
- `Dialog::DropMenu` carries that point (`at`). `dialogs::view` draws it not as a dialog but as a
  window-sized layer: a libcosmic `popover` with `Position::Point(at)` (it keeps the menu inside
  the window, flipping it above the point near the bottom edge) and a menu card
  (`Container::Dropdown`, `menu_button` rows: the title line, Copy, Move, Link, Cancel).
  A press outside the card cancels, as Esc.
- No point (a drop seen without `on_drop`): the old centred dialog.
- Picking an item works as before: `DropAs(op)` → the F5 / F6 / link dialog aimed at the dir.
