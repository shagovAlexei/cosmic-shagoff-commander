# 67. Drag & drop — план

Спек: `.claude/docs/specs/67-drag-drop.md`.

1. Ядро (`panel.rs`, тесты сначала):
   - `Panel::drop_dir(Option<i>)`: каталог-строка → `cwd/имя` (`..` → родитель); иначе `cwd`.
   - `drop_is_noop(sources, dir)`: все источники уже лежат в `dir`, или `dir` — один из них.
2. `clip.rs`: `drag(child, paths, label)` — `dnd_source` с `Files` (uri-list + gnome) и иконкой
   с именем / «N файлов»; `drop_zone(child, on)` — `dnd_destination_for_data::<Paste>`,
   действие по умолчанию Copy.
3. `app.rs`: `Message::Drop { side, dir, paths, mv }` → `Dialog::Input` Copy / Move с
   `dir_input(drop_dir)`; игнор при задаче / диалоге / результатах / noop.
   `Message::DropHover(Option<(side, row)>)` → `App.drop_hover` (подсветка строки).
4. `view.rs`: строки обёрнуты в `drag` (не в архиве); строки-каталоги — в `drop_zone`;
   список панели целиком — в `drop_zone` (не в результатах).
5. i18n `dnd-files`; tc-reference «Мышь»; TESTING 67; ROADMAP; версия 0.11.0.
6. Проверка: `just verify`, headless `vpointer drag` между панелями.
