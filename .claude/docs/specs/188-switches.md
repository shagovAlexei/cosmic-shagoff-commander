# 188. Список исключений и переключатели

TC: `cm_SwitchIgnoreList`, `cm_SwitchWatchDirs`, `cm_SwitchDirSort`, `cm_SwitchColorsByFileType`, `cm_SrcUnsorted` (Ctrl+F7).

- Настройки (`Config`, все с умолчаниями): `ignore_on` (нет), `ignore_list` (строка масок через `;`, `mask::Mask` как у Ctrl+F12), `watch_dirs` (да), `dirs_by_name` (нет), `type_colors` (да). Ctrl+, → Панели и Вид → «Переключатели» ▸ (флажки); действия `Switch*` переключают настройку через `App::set`, как Ctrl+H.
- Список исключений и «каталоги по имени» — на всю программу, как сама настройка: core держит их глобально (`panel::set_ignore`, `sort::set_dirs_by_name`, ставит `Config::set_core_switches` в `build` и `apply_config`), иначе каждую новую вкладку пришлось бы настраивать. Смена — `Panel::refresh` во всех вкладках. Исключённые имена отсеиваются в `rebuild`, как быстрый фильтр.
- Без наблюдения (`watch_dirs` выкл.) подписка `watcher` не создаётся.
- Цвета по типам выкл.: `Config::scheme` без правил, цвета панели остаются.
- `SortKey::Unsorted` (Ctrl+F7, меню «Вид»): сравнение `Equal`, стабильная сортировка оставляет порядок `scan` (порядок каталога); каталоги сверху. Обратного порядка у него нет.
- Не делаем: список исключений файлом, как в TC.
