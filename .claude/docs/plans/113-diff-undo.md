# 113 — Отмена переноса блока: план

Спек: `../specs/113-diff-undo.md`.

1. `DiffDlg.undo: Vec<(Arc<(String, String)>, (bool, bool))>` — тексты и `dirty` до переноса; кладётся в `DiffReady`, если перенос изменил текст.
2. `Message::DiffUndo`: снять верхний, вернуть `dirty`, пересчитать строки из этих текстов (`start_diff_with` с третьим аргументом — тексты). `DiffSave` очищает `undo`. Тест в `app.rs` рядом с `diff_copy_block_then_save_and_esc_asks_first`.
3. Кнопка «Отменить» (`diff-undo`, en / ru), tc-reference, TESTING § 113, ROADMAP, 0.56.0.
