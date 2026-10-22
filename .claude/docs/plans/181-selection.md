# План 181: выделение

1. core `panel`: `Like` + `mark_like` (`mark_ext` через него), `keep_marks` публичный, `marked_names`, `mark_listed`, `goto_marked`, `show_only_marked` (поле `only`, сброс в `set_listing` / `set_show`); тест `selection_by_name_saved_listed_shown_only_and_walked`.
2. app: 11 `Action`, `InputOp::SaveMarks` / `LoadMarks` (`dialog_for`, `submit_dialog`), `Message::MarksText` (`clipboard::read`); меню «Выделение» / «Вид», кнопки `cm_*`, en/ru; тест `the_selection_goes_to_a_file_and_back`.
3. tc-reference, ROADMAP, TESTING, 0.95.0; verify, headless, `/code-review`, PR.
