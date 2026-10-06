# 60. План: очистка истории

Спек: `.claude/docs/specs/60-history-clear.md`.

1. core (TDD): `History::clear` — остаётся текущий каталог.
2. app: `Message::ListClear` (кнопка в окнах History / Commands); `Config.keep_history` +
   `Setting::KeepHistory` + переключатель; `save_state` пишет пустые истории, если выключено.
3. Версия 0.4.0; i18n; TESTING.md, ROADMAP.md, tc-reference.md.
4. `just verify`, headless, `/code-review`, PR.
