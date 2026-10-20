# 169. Folder tree (Alt+F10) — plan

Spec: `.claude/docs/specs/169-tree-dialog.md`.

1. core `tree.rs`: `Tree` + `chain` + `subdirs` / `subdirs_toward`, tests.
2. app: `Action::TreeDialog` (Alt+F10, Commands menu, F1), `Dialog::Tree`, keys / letters / pick,
   `tree_read`, `Message::TreeFilled`, submit → `load`. Test.
3. tc-reference, ROADMAP, TESTING, CLAUDE.md, version 0.83.0.
