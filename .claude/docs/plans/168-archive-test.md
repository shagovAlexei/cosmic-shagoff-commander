# 168. Test archives — plan

Spec: `.claude/docs/specs/168-archive-test.md`.

1. core `archive`: `files(strict)` shared with `for_each_file`; `test` (+ single-file streams).
   Test: four formats packed, a flipped byte and a cut file fail, a .gz passes.
2. app: `Action::TestArchives` (Alt+Shift+F9, Files menu, F1), background run, `Message::Tested`.
3. tc-reference, ROADMAP, TESTING, version 0.82.0.
