# 112 — Формат размера: план

Спек: `../specs/112-size-format.md`.

1. `Config.human_sizes: Option<bool>` + `Config::human_sizes()` (по теме, если не выбрано); `Setting::HumanSizes`, `Action::ToggleHumanSizes` — пункт меню «Вид» и переключатель в настройках. Тест.
2. `view::row` — колонка, `view` итоги (`size-bytes` вынесен из шаблона `status`).
3. tc-reference, TESTING § 112, ROADMAP, 0.55.0.
