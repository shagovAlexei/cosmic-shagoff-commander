# 61. План: горячие буквы `&` в избранном

Спек: `.claude/docs/specs/61-hotlist-hotkeys.md`.

1. core (TDD): `quicksearch::hotkey(name) -> (shown, Option<(at, key)>)`.
2. dialogs: `menu_row_with(icon, hot, …)`, подчёркнутая буква через `rich_text`; для `ListKind::Hotlist`
   показывать без `&`.
3. app: в `CmdType` для Ctrl+D — единственная горячая буква → `pick`, несколько → курсор по ним.
4. Версия 0.5.0; TESTING.md, ROADMAP.md, tc-reference.md.
