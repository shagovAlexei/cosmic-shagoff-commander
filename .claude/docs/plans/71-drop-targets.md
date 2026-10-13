# 71. Бросок на вкладки и кнопки дисков — план

Спек: `.claude/docs/specs/71-drop-targets.md`.

1. Тест `drop_on_a_tab_or_drive_button_works_beside_search_results`; в `Message::Drop` проверка
   результатов поиска — только без `dir`.
2. `view::drop_into`; зоны на вкладках (`tab_bar`) и кнопках дисков (`drive_bar`).
3. TESTING 71 (руками: headless sway не перетаскивает), tc-reference, ROADMAP, версия 0.14.0.
