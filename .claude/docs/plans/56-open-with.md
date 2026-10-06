# 56. План: «Открыть с помощью»

Спек: `.claude/docs/specs/56-open-with.md`.

1. `core::openwith` (TDD): `parse_content_type`, `parse_mime` (default первым, без повторов),
   `parse_desktop(text, lang)` → имя или `None` при `Hidden=true`, `find_desktop(id, dirs)`,
   `apps(file, lang)`, `launch_argv(desktop, files)`.
2. `keymap::Action::OpenWith` без клавиши; пункт «Открыть с помощью…» в `menu::context_items` для
   файла (не каталог); в `every_item_shows_a_key` — не нужен (только контекстное меню).
3. `app`: `Action::OpenWith` → проверка архива → `spawn_blocking(apps)` → `Message::OpenWithApps`
   → `Dialog::List { kind: OpenWith }`; `pick` → `gio launch` с `targets()`.
4. i18n en/ru: `menu-open-with`, `open-with`, `open-with-none`, `open-with-archive`.
5. `tc-reference.md` (строка правого щелчка), `TESTING.md`, `ROADMAP.md` (+ строки: проверка на
   X11 / Ubuntu / Linux Lite, сборка под старую glibc).
6. `just verify`, headless-проверка меню и окна, `/code-review`, PR.
