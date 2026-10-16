# 102. The panels' font in list dialogs

ROADMAP «Шрифты, дальше: диалоги-списки (Избранное Ctrl+D, Alt+F1, история)».

- `dialogs::menu_row(_with)` take `ListFont` = `App::panel_font()` (Settings → «Шрифты»): the list
  dialogs (Alt+F1, Alt+F2, Alt+↓, Ctrl+D and its «Настроить…», «Открыть с помощью», the
  toolbar and colour-rule editors) draw their rows in it; the path in it 2 pt smaller; the row is
  twice the size high, never below the former 28 px (program icons are 20 px).
- The drop menu keeps the interface font: a menu with a fixed-width card.
- Menus drawn by libcosmic (main / context) keep the interface font.
