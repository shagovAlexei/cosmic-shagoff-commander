# 102. The panels' font in list dialogs

ROADMAP «Шрифты, дальше: диалоги-списки (Избранное Ctrl+D, Alt+F1, история)».

- `dialogs::menu_row(_with)` take `ListFont` = `App::panel_font()` (Settings → «Шрифты»): the list
  dialogs (Alt+F1, Alt+F2, Alt+↓, Ctrl+D and its «Настроить…», «Открыть с помощью», the
  toolbar and colour-rule editors, the drop menu) draw their rows in it; the path in it 2 pt
  smaller; the row is twice the size high (28 at the default 14).
- Menus drawn by libcosmic (main / context) keep the interface font.
