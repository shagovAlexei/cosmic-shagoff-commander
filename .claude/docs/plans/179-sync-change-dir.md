# План 179: синхронная смена каталогов

1. core: `panel::sync_step(from, to, other, has)` + тест `sync_step_follows_down_and_up` (вниз / нет каталога / вверх / мельче / корень / в сторону / через архив).
2. app: `Action::SyncChangeDir`, `Action::MatchSrc`; `App.sync_dirs`; в `App::load` для активной стороны — `sync_step` и `load` другой; тест `sync_change_dir_takes_the_other_panel_along`.
3. Меню «Команды» (галочка `Checks.sync_dirs`), справка F1, кнопки `cm_SyncChangeDir` / `cm_MatchSrc`, строки en/ru.
4. tc-reference, ROADMAP, TESTING, версия 0.93.0; `just verify`, headless, `/code-review`, PR.
