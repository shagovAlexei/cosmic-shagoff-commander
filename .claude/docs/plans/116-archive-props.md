# 116 — Свойства в архиве: план

Спек: `../specs/116-archive-props.md`.

1. core `archive::usage_of(archive, inners)`. Тест рядом с `usage_counts_from_the_index`.
2. app: `Props.archive: Option<PathBuf>`; `App::archive_props` из строк панели; `start_usage` — по оглавлению; `PropsBit` в архиве не действует; `read_only` больше не запрещает `Properties`. Вид: флажки без `on_toggle`, без группы, «Закрыть». Тест.
3. Headless: каталог и файл в zip. tc-reference, TESTING § 116, ROADMAP, 0.59.0.
