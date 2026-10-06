# 66. План: ветвь Ctrl+B

Спек: `.claude/docs/specs/66-branch-view.md`.

1. core (TDD): `listing::branch(dirs, show_hidden)` — файлы на любой глубине, без захода по ссылкам.
2. keymap Ctrl+B / Ctrl+Shift+B, меню «Вид», справка, `cm_DirBranch` / `cm_DirBranchSel`.
3. app: `Tab.branch`; `App::branch` → фон → `BranchListed` (проверка, что вкладка ещё там) →
   `results` + `reload`; повтор — выход; заголовок «Ветвь: …».
4. README: раздел «Update»; версия 0.10.0; TESTING, ROADMAP, tc-reference.
