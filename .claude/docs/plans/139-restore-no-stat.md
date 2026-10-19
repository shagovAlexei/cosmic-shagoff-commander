# 139 — план

1. `restore` без `existing_dir`; тест `restore_clamps_active_and_fixes_paths_without_the_disk`.
2. `Tab.restored` (ставится в `build`), `load_tab` берёт его и решает путь в фоне.
3. `build_restores_tabs_and_active_pane`: пропавший каталог не трогается при старте.
