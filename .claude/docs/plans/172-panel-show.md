# 172. Which files a panel shows — plan

Spec: `.claude/docs/specs/172-panel-show.md`.

1. core `panel`: `Show`, `set_show` / `show`, `rebuild` filters by it under the quick filter.
   Test: programs, a mask with `|.git\`, kept in another dir, with the quick filter on top.
2. app: `Action::ShowAll` / `ShowPrograms` / `ShowMask` (Ctrl+F10 / F11 / F12), `MaskOp::Show` in
   the mask dialog, status line, View menu, F1.
3. tc-reference, ROADMAP, TESTING, version 0.86.0.
