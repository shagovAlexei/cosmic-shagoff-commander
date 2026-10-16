# 108 — Линия и рамка: план

Спек: `../specs/108-pane-lines.md`.

1. `Config.divider_line`, `Config.active_frame` (false); `Setting::DividerLine / ActiveFrame`; переключатели в «Интерфейс» (рамка — только в классическом); i18n.
2. `Action::ToggleDividerLine / ToggleActiveFrame` → `App::set(Setting…)`; флажки в меню «Вид» (`menu::bar` получает конфиг).
3. view: линия 1 px по центру промежутка; классическая панель — `container` с рамкой акцента у активной (прозрачной у неактивной), отступ 1 px.
4. Тест через `update`: пункты меню переключают конфиг.
5. tc-reference, TESTING § 108, 0.51.0.
