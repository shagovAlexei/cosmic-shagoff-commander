# 69. Очередь операций: исправления — план

Спек: `.claude/docs/specs/69-queue-fixes.md`.

1. Тесты `regression_a_queued_copy_is_planned_when_it_starts`,
   `regression_closing_the_window_asks_while_a_job_runs_or_waits`,
   `regression_a_background_question_waits_for_the_open_dialog`.
2. `Queued.replan`, `start_queued`; `finish_job` пересчитывает `ops::plan`.
3. `exit_on_close(false)`, `on_app_exit`, `close_requests` → `Message::Quit`; `Dialog::ConfirmExit`,
   `App.quitting`; строки en / ru.
4. `App.held` + `ask()`; `update` показывает отложенный вопрос.
5. headless: диалог выхода при копировании, «Прервать и выйти» — нет `.shagoff-part`.
6. TESTING 69, tc-reference, ROADMAP, версия 0.12.1.
