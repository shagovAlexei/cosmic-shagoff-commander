# 176. Edit path — plan

1. core `cmdline::resolve` (from `parse`) + test.
2. app: `Action::EditPath`, `Message::PathEditStart/PathEdit/PathEditGo`, `App.path_edit`, `edit_path`,
   Rename on `..` → edit, Esc, `load` clears; view: field in place of the line, `mouse_area` around
   it for the free space; menu, help, i18n; test.
3. Headless, docs, 0.90.0.
