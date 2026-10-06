# 57. План: «Открыть с помощью» — значки, другая программа, по умолчанию

Спек: `.claude/docs/specs/57-open-with-more.md`.

1. core (TDD): `parse_desktop` → `Entry { name, icon }`; `Found { mime, apps }`; `other_line`;
   `set_default`.
2. dialogs: `ListItem.icon`, значок в `menu_row`; `Item::Other`; `InputOp::OpenWith`; кнопка
   «По умолчанию» для `ListKind::OpenWith`.
3. app: строка «Другая программа…», ввод команды → `spawn_in(cmdline::argv(..))`;
   `Message::OpenWithDefault` → `blocking(set_default)` → сообщение.
4. i18n en/ru, TESTING.md, ROADMAP.md, tc-reference.md.
5. `just verify`, headless, `/code-review`, PR.
