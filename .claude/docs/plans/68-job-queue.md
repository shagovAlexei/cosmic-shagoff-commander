# 68. Очередь операций — план

Спек: `.claude/docs/specs/68-job-queue.md`.

1. `App.queue` + `Queued { side, kind, job, focus }`; `start_job`: занято → в очередь + «В очереди: N».
2. `finish_job`: после перечитывания панелей — следующая из очереди, `hidden = true`.
3. `open_from_archive`: занято → «Идёт другая операция» (не в очередь).
4. `Message::DialogQueue`: `submit_dialog`, новую задачу — в фон; F2 (`FieldKey(Rename)`) в
   диалоге Copy / Move; кнопка «F2 В очередь» (`tertiary_action`).
5. `job_line`: «(ещё в очереди: N)».
6. Тесты: вторая задача в очереди и стартует в фоне после первой; F2 в диалоге — в фон.
7. tc-reference, TESTING 68, ROADMAP, версия 0.12.0.
