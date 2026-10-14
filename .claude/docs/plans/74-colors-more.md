# 74. Цвета: фон, размер, возраст — план

Спек: `.claude/docs/specs/74-colors-more.md`.

1. core `colors.rs`: `Rule { bg, size, age }`, `bound`, `Look`, `Rule::valid`, `Scheme::pick(e, now)` + тесты.
2. `view.rs`: `cursor_style(.., bg)` — фон и чёрный / белый текст; имя — `Look.fg`.
3. `colorrules.rs`: поля размера и возраста, «Текст» / «Фон», `⚠` у неверных + тест.
4. headless (X11): окно при 1036×530; TESTING 74, ROADMAP, tc-reference, версия 0.17.0.
