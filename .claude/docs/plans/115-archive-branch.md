# 115 — Ветвь в архиве: план

Спек: `../specs/115-archive-branch.md`.

1. core `archive::branch(archive, inner, hidden)` — пути файлов; `archive::entries(paths, root)` — строки за один проход по оглавлению; общий `to_entry` с `list`. Тест на zip.
2. app: `read_listing` для результатов внутри архива — `archive::entries`; `App::branch` без запрета, каталоги Ctrl+Shift+B — по строкам панели, чтение — `archive::branch`. Тест.
3. Headless: F3, F5 наружу, F8 из ветви. tc-reference, TESTING § 115, ROADMAP, 0.58.0.
