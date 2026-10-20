# 170. Tree in the panel (Ctrl+F8)

TC Ctrl+F8: the active tab shows the dirs as a tree instead of its files; the other panel shows
the files of the dir under the tree's cursor. Ctrl+F8 again — the files back.

- The tree is `core::tree::Tree` (169), kept on the tab (`Tab.tree`: the id of its reads + the
  tree); opened from `/` down to the tab's dir (the way through hidden dirs shown), reads off the UI
  thread (`tree_read` → `Message::TreeFilled`, by id: a panel's tree or the Alt+F10 dialog).
- Drawn in the panel's place, the rows as the file list draws them (only those in view,
  `viewport::visible_range`), no column header; the cursor kept in view (`reveal`).
- Keys (`App::tree_act`): ↑ ↓ PgUp PgDn Home End, → open / step in, ← close / step out, Alt+letter
  — the next dir with it; each move takes the other panel there (`tree_follow`). Enter — focus to
  the other panel (the files are there). Tab, F1, Ctrl+, and Alt+F10 work as usual; file operations
  do nothing (the files under the tree are not on screen).
- Mouse: a click picks a row (the other panel follows), a double click opens / closes it.
- A new tab (Ctrl+T) shows the files.
