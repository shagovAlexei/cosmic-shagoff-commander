# План 180: меню «Запуск»

1. `Config.start_menu: Vec<ToolButton>` (по умолчанию пусто).
2. `Action::ConfigureStartMenu`, `Action::UserMenu(i)`; `menu::Checks.start` — названия пунктов; меню «Запуск».
3. `ToolEdit::menu` (`menu: bool`): без значков, «Сброс» не действует и не показан; `submit_dialog` пишет `start_menu`.
4. Кнопка `cm_ChangeStartMenu`, справка F1, строки en/ru.
5. Тест `the_start_menu_is_edited_apart_from_the_bar_and_runs_its_items`; tc-reference, ROADMAP, TESTING, CLAUDE.md, 0.94.0; verify, headless, `/code-review`, PR.
