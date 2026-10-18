# 123 — план

1. `ops::delete_kept` + тест `delete_kept_leaves_what_did_not_go_in`.
2. `Change::Add.move_sources`; `apply` отдаёт пары (источник, копия); `rewrite` после rename — `delete_kept` по копиям. Тест `regression_move_into_archive_keeps_files_that_appeared_meanwhile` (красный на старом коде).
3. `archive::packed`; `Job::Pack` с `move_after` — `delete_kept` по записям архива. Тест `pack_moving_deletes_what_went_in` (zip, tar.gz, 7z, с паролем).
