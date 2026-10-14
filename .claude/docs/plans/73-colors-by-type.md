# 73. Цвета по типам файлов — план

Спек: `.claude/docs/specs/73-colors-by-type.md`.

1. core `colors.rs`: `Rule`, `What`, `parse`, `pick`, `defaults` + тесты.
2. `Config.colors`; `view::name_cell` берёт `Paint` (полные строки и Brief).
3. `colorrules.rs`: `ColorEdit` / `ColorMsg` / `view` + тест; `Dialog::Colors`, `Message::Colors`,
   `Action::ConfigureColors` (меню «Конфигурация»); сохранение `set_colors` по OK; тест в app.
4. headless (X11: меню внутри окна): окно при 1036×530; TESTING 73, ROADMAP, версия 0.16.0.
