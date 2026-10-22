# План 187: сравнение каталогов — подкаталоги, только различия

1. core: `sync::compare_lists(.., dirs)` + проверка в `compare_lists_marks_unique_and_newer`.
2. app: `Action::CompareListsSubdirs` / `CompareListsDiffOnly` (одна ветка с `CompareLists`), подменю, справка, кнопки, `on_files`, отказ в результатах; тест `compare_with_subdirs_marks_dirs_and_diff_only_hides_the_same`.
3. tc-reference, ROADMAP, TESTING, 0.101.0; verify, `/code-review`, PR.
