# 109 — Кодировки: план

Спек: `../specs/109-lister-encodings.md`.

1. core: `encoding_rs`, `chardetng`; `Encoding::Other(&'static encoding_rs::Encoding)`, `Encoding::of`; `decode_as` через `encoding_rs` (таблицы удалить); `guess`: `chardetng` → кириллица — наш счётчик, иначе найденная. Тесты: 1252, 1250, Shift_JIS, 1253; русские — как были.
2. app: `ENCODINGS` / `ENCODING_NAMES` — `LazyLock`, наши + остальные.
3. tc-reference, CLAUDE.md, TESTING § 109, ROADMAP, 0.52.0.
