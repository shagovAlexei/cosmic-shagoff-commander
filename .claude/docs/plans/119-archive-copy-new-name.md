# 119 — план

1. Тест `regression_one_file_to_a_new_name_inside_is_a_file_not_a_dir` (repack): `sub/renamed.txt` → файл, `fresh/` → каталог.
2. `apply` (`Change::Add`): правило TC для одного источника.
3. `start_transfer`: `/` из ввода в `inner`; в `cross` внутри одного архива — один файл на самого себя → `SameFile`.
