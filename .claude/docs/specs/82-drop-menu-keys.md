# 82. Keys in the drop menu

ROADMAP «Drag & drop, дальше: ходить по меню броска стрелками и выбирать буквой».

TC's drop menu is a Windows popup menu: arrows move, Enter picks, an underlined letter picks at once.

- `Dialog::DropMenu.cursor`: the row under the cursor — an index into `dialogs::DROP_OPS`
  (Copy, Move, Link), one past them is Cancel. Opens on Copy.
- ↑ / ↓ move it (wrapping, as a Windows menu); Enter picks the row (Cancel closes); Esc closes.
- Hot letters: the item names carry `&` (`&Копировать`, `&Переместить`, `&Ссылка`; en `&Copy`,
  `&Move`, `&Link`) parsed by `quicksearch::hotkey`, underlined; a typed letter (`CmdType`) picks
  its item at once. The translation chooses the letter.
- Rows are `menu_row_with` (the cursor colour of the other TC menus). A row without a path now
  gives the name the whole width (it was cut at 2/5 — also in Ctrl+D submenu rows).
