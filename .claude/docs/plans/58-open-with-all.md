# 58. План: «Открыть с помощью» — все программы, история команд

Спек: `.claude/docs/specs/58-open-with-all.md`.

1. core (TDD): `parse_desktop` отбрасывает `NoDisplay` и не-`Application`; `all_apps(dirs, lang)`.
2. app: `Item::All`, `open_with_items(apps, all)`, `Message::OpenWithAll`; `State.other_cmds` +
   `App.other_cmds`, `remember` при запуске, открытие с последней, ↑ / ↓ через `mask_history`.
3. i18n, TESTING.md, ROADMAP.md, tc-reference.md, CLAUDE.md.
4. `just verify`, headless, `/code-review`, PR.
