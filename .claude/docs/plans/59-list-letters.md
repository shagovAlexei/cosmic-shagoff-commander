# 59. План: буквы в окнах-списках

Спек: `.claude/docs/specs/59-list-letters.md`.

1. core (TDD): `quicksearch::next_with(labels, from, c)`.
2. app: `Message::CmdType` при открытом `Dialog::List` → `next_with` → курсор + `list_snap`
   (общий с ↑ / ↓).
3. Версия 0.3.0; TESTING.md, ROADMAP.md, tc-reference.md.
4. `just verify`, headless, `/code-review`, PR, тег после мержа.
