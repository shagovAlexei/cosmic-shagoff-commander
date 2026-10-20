# 167. TC keys — plan

Spec: `.claude/docs/specs/167-tc-keys.md`.

1. core `panel`: `restore_marks` (saved by `unmark` / `mark_all(false)`), `mark_ext(on)`. Tests.
2. app keymap: Num/, Alt+Num±, Shift+F10 / ContextMenu, Ctrl+← / →, Ctrl+L → new `Action`s;
   menu key table and help rows.
3. app: Shift+F10 list dialog (`ListKind::Context`, `Item::Act`); Ctrl+← / → through `load`;
   Ctrl+L `props::usage` in the background → status. Tests.
4. tc-reference, ROADMAP, TESTING, version 0.81.0.
