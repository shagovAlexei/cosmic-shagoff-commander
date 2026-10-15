# 82. Keys in the drop menu — plan

Spec: `../specs/82-drop-menu-keys.md`.

1. `Dialog::DropMenu { cursor }`, `dialogs::DROP_OPS`, `drop_label(op)`; ftl names with `&`.
2. `Message::Key` with the drop menu: Up / Down wrap over 4 rows, Enter → `DropAs` or cancel.
3. `Message::CmdType(c)` with the drop menu: the item whose `hotkey` letter is `c` → `DropAs`.
4. `drop_menu` view: `menu_row_with` rows with the cursor and the underlined letter;
   `menu_row_with` without a path: the name fills the row.
5. Test: extend `right_button_drop_asks_copy_move_or_link` (↑ wraps, ↓ ↓ Enter = Move, `L` = Link).
