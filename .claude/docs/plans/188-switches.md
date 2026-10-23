# План 188: список исключений и переключатели

1. core: `SortKey::Unsorted`, `sort::set_dirs_by_name` + тест `unsorted_keeps_the_dir_order_and_dirs_may_go_by_name`; `panel::set_ignore`, фильтр в `rebuild`, `Panel::refresh`.
2. app: поля `Config`, `Config::scheme` / `set_core_switches`, `Setting::*`, настройки, `Action::Switch*`, Ctrl+F7, меню «Вид» ▸ «Переключатели», справка, кнопки, подписка `watcher`; тест `switches_flip_their_settings_and_the_ignore_list_hides_its_names`.
3. tc-reference, ROADMAP, TESTING, 0.102.0; verify, `/code-review`, PR.
