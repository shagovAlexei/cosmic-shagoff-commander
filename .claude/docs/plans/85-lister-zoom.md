# 85. Lister zoom — plan

Spec: `../specs/85-lister-zoom.md`.

1. Tests first: `config::size_step` bounds; `lister_key` for `+` / `=` / `-` (and German `+`).
2. `ListerKey::Zoom(bool)`; `lister_key` by Code::Equal / Minus or the typed character.
3. `App::lister_zoom` → `set(Setting::ListerSize)`; Num+ / Num− in `lister_action`.
4. App test: Zoom and Num± change `config.lister_size` and the open viewer's size.
5. headless: F3, `===`, `-` → size saved.
