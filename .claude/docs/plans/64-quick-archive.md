# 64. План: Ctrl+Q внутри архива

Спек: `.claude/docs/specs/64-quick-archive.md`.

1. core (TDD): `archive::extract_one` — тихий `Handler` (конфликт / ошибка → отмена, `cancelled` = флаг).
2. `quick_follow`: внутри архива файл → `fresh_temp_dir` → `extract_one` → `Loaded::read` →
   `remove_dir_all`; каталог — пояснение.
3. Тест app; версия 0.8.0; TESTING, ROADMAP, tc-reference.
