# 155 — план

Спек: `specs/155-nested-archives.md`.

1. `How::Browse` / `Open::Browse(back)`: Enter на архиве в архиве — извлечь и `load` копии вместо перечитывания; `App::nested`.
2. `shown` / `real`: путь копии ↔ путь через внешний архив (вид — `view::pane`, `command_line`; `load` — `real`).
3. Backspace и `..` в корне копии — во внешний (`Action::Parent`, `Action::Enter`).
4. Только чтение: `nested_change` (до диалога), `changes_nested` (в `start_queued`). Тест; строки en / ru; tc-reference, CLAUDE.md, ROADMAP, TESTING § 155; 0.69.0.
