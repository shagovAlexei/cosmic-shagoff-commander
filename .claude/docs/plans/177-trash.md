# 177. План: корзина

1. core `trashcan.rs`: `home`, `is_top`, `inside`, `origin` (unescape, корень диска), `restore`, `forget`, `everything` — тесты.
2. `listing::scan`: `target` = исходный путь в корзине; `ops::delete`: `forget` после удаления.
3. app: `Action::{OpenTrash, Restore, EmptyTrash}`, `Message::Restored`, F8 в корзине — безвозвратно,
   `menu::Ctx::Trash` + `Ctx::entry`, пункт в «Команды», кнопка `cm_OpenRecycled`, строки en/ru, справка.
4. headless: восстановление, F8 (без `.trashinfo`), меню корзины; `headless.sh` — свой `XDG_DATA_HOME`.
5. Документы: tc-reference, ROADMAP, TESTING, CLAUDE.md; версия 0.91.0.
