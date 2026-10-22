# План 184: новый файл по шаблону

1. core `templates`: `dir`, `list`, `create` + тест `the_dir_comes_from_user_dirs_and_files_are_copied_once`.
2. app: `Action::NewFromTemplate` (меню «Файлы», справка, `cm_EditFileMenu`), `ListKind::Templates` → `InputOp::FromTemplate` → `submit_dialog`; `read_only`; тест `a_new_file_from_a_template_is_its_copy_never_over_a_file`.
3. tc-reference, ROADMAP, TESTING, 0.98.0; verify, `/code-review`, PR.
