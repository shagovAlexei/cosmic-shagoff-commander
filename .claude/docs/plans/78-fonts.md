# 78. Fonts — plan

1. `config.rs`: `panel_font`, `panel_size`, `lister_size`, `SIZES`, `Config::row_h` (+ test).
2. `app.rs`: `face` (`font_named`, leaked name once per change), `panel_font()`, `row_h()` from config; `apply_config` rebuilds the face, resizes an open lister, rescales offsets on a row height change.
3. `view.rs`: `panel_cell`, `name_cell(.., font)`, `brief_list(app, ..)`.
4. `lister.rs`: `size`, `row_h()`, `mono_w()`, `step_x()` instead of constants.
5. `drawer.rs`: «Fonts» section, `font_families()`, `Setting::{PanelFont, PanelSize, ListerSize}`.
6. i18n, ROADMAP, TESTING § 78, version 0.21.0, headless check.
