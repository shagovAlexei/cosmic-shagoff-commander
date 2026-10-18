# 117 — Остановка второго шага ветви: план

Спек: `../specs/117-branch-stop-stat.md`.

1. core `listing::entries(paths, root, stop)`. Тест.
2. app: `read_listing(.., stop)`; `load_tab` даёт `branch_stop`, пока `branching`; `BranchListed` не снимает `branching` у показанной вкладки; `Listed` снимает (и «Чтение…»); Esc: у вкладок на втором шаге `results = None` и обычное чтение каталога. Тест.
3. Headless на `/usr` (Ctrl+B через `vkey`). tc-reference, TESTING § 117, ROADMAP, 0.60.0.
