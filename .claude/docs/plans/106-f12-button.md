# 106 — F12: план

Спек: `../specs/106-f12-button.md`.

1. `Config.f12: ToolButton` (по умолчанию пустая), `keymap`: F12 → `Action::F12`; справка; i18n en/ru.
2. `App::run_tool(i)` → `run_button(&ToolButton)`; `Action::F12`: пустая — `Action::Settings`, иначе `run_button`.
3. Настройки: раздел «F12» — dropdown («Своя команда» + `builtins`), поля программы и параметров (только для своей); `Setting::F12(usize) / F12Cmd / F12Params`.
4. `view::fkey_bar` — двенадцатая кнопка с подписью `toolbar::tip`.
5. Тесты через `update`: SC-команда выполняется, пустая открывает настройки, выбор в настройках пишет конфиг.
6. tc-reference, TESTING § 106, ROADMAP, 0.49.0.
