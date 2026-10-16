# 107 — Lister в окне: план

Спек: `../specs/107-lister-window.md`.

1. `Config.lister_window: bool` (true), `Setting::ListerWindow`, переключатель, i18n.
2. `Lister.window: Option<window::Id>`; `open_lister`: новый просмотр при настройке — `window::open` + заголовок; уже открыт в окне — `gain_focus` + заголовок.
3. `App::viewer()` — просмотр в панели, либо окно просмотра в фокусе (`core.focused_window()`; нет сведений — считать в фокусе); `viewer_shown()` — `None` для окна; view.rs — панели не заменяются.
4. `view_window(id)`: окно просмотра — `header_bar` (title, on_close, on_drag → `window::drag`, on_maximize / двойной щелчок → `toggle_maximize`) + `lister::view`; чужой id — пусто (не паника).
5. `ListerClose` — `window::close`; `on_close_requested` / `Escape(window)` для окна просмотра — `ListerClose`; событие `Closed` окна просмотра — `lister = None`.
6. Тесты: F3 открывает окно (`Lister.window`), клавиши главного окна идут панелям, закрытие; при выключенной настройке — как раньше.
7. tc-reference, TESTING § 107, CLAUDE.md, 0.50.0.
