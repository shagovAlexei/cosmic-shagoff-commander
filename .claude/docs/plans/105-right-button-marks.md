# 105 — Правая кнопка отмечает: план

Спек: `../specs/105-right-button-marks.md`.

1. `Config.right_marks: bool` (по умолчанию `true`), `Setting::RightMarks`, переключатель в «Панели», i18n en/ru.
2. `App.right_hold: Option<Hold>` (поколение, сторона, строка, `armed`). `RightClick` / `RightEmpty` при `right_marks`: курсор, `Hold`, задача `sleep(500 ms)` → `Message::RightHeld(gen)`; `RightHeld` — `armed`, подготовка меню как раньше; `route_event`: отпускание правой → `Message::RightUp` → не `armed` — `toggle_mark`; `DragStart` снимает `Hold`.
3. view: `context_menu` получает меню, только если `!right_marks` или `Hold.armed`.
4. Тесты через `update`: короткий щелчок отмечает, долгий — меню без отметки, перетаскивание — ничего, режим меню — как раньше.
5. tc-reference, TESTING § 105, ROADMAP, 0.48.0.
