# 104 — Эскизы: план

Спек: `../specs/104-thumbnails.md`.

1. core `viewport`: `thumb_rows(len, cols)`, `thumb_range(len, cols, cell_h, offset, height)` — видимые записи; тесты.
2. core `session::PaneState.thumbs` (`serde(default)`).
3. app `keymap`: `Action::ViewThumbs` на Ctrl+Shift+F1; меню, справка, `cm_SrcThumbs`, i18n en/ru.
4. app `Tab.thumbs`; `act`: ←→ ±1, ↑↓ ±cols, страница = ряды экрана × cols; `reveal` — `scroll_to_cursor` по ряду; `Resized` — перестроить.
5. app `thumbs.rs`: `make(path) -> Option<Handle>` (decode из `lister`, `thumbnail(96, 96)`); тест на 2000×1000.
6. app: `App.thumbs` кеш + `thumbs_busy`; после каждого `update` — `want_thumbs()` запрашивает видимые; `Message::Thumb`.
7. view: `thumb_list` — виртуальная сетка в scrollable панели.
8. tc-reference, TESTING § 104, ROADMAP, версия 0.47.0.
