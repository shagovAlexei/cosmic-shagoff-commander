# 72. Ссылки и перетаскивание правой кнопкой — план

Спек: `.claude/docs/specs/72-links-right-drag.md`.

1. core: `ops::symlinks` + тест `symlinks_point_at_the_sources_and_never_replace`.
2. app: `Action::Link` (keymap, menu, help), `InputOp::Link`, `App::make_links`; тест
   `ctrl_shift_f5_links_the_file_into_the_other_pane`.
3. `dnd.rs` (копия `dnd_source` с правой кнопкой), `clip::drag` на ней; `Message::DragStart`,
   `Dialog::DropMenu`, `Message::DropAs`; тест `right_button_drop_asks_copy_move_or_link`.
4. headless: Ctrl+Shift+F5 (перетаскивание — руками); TESTING 72, tc-reference, ROADMAP, CLAUDE.md,
   версия 0.15.0.
