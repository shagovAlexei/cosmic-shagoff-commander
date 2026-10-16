# 111 — Просмотр файлов больше 32 МБ: план

Спек: `../specs/111-big-files.md`.

1. core `lister`: `Doc { start, bom }`, `load_at(path, from, to, line, bom)` (начало строки / кратно 16, UTF-16 по BOM первого окна), `newlines` (концы строк в сырых байтах до смещения), `line_row` (первая строка экрана k-й строки файла), `last_line_start`. Тесты.
2. app `Lister`: `file`, `pending: Option<Slide>`, `want_window()` после прокрутки, `set_window()` с пересчётом прокрутки; Home / End; поиск через окна. `Message::ListerWindow`.
3. Подпись «Показано a–b из c» (en / ru), tc-reference, TESTING § 111, ROADMAP, 0.54.0.
