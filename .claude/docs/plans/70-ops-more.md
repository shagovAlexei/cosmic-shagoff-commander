# 70. Операции: маска, «Переименовать старый», порядок очереди — план

Спек: `.claude/docs/specs/70-ops-more.md`.

1. core: тесты `mask_name_like_tc`, `plan_with_a_mask_renames_each_source_in_that_dir`,
   `conflict_rename_old_moves_the_existing_one_aside`; `mask_name`, маска в `plan`,
   `Resolution::RenameOld` (ops + archive).
2. app: `Action::CopySame` при отметках → `*.*` (тест); кнопка «Переименовать старый», «Отмена» —
   `secondary_action`; `Message::QueueUp` + ↑ в окне прогресса (тест в тесте очереди).
3. headless: конфликт, окно очереди, Shift+F5 → `*.bak`.
4. tc-reference, TESTING 70, ROADMAP, версия 0.13.0.
