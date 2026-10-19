# 151 — план

Спек: `specs/151-shift-enter.md`.

1. keymap: Shift+Enter → `Action::RunTerminal` (тест).
2. app: из панели с набранной строкой — `cmd_run(true)`; иначе в `act` — файл под курсором через `cmdline::argv` + `terminal_prefix`, папка — `Enter`, архив — ошибка. Тест.
3. Справка, tc-reference, ROADMAP, TESTING § 151; 0.67.0.
