# 156 — план

Спек: `specs/156-zip-fast.md`.

1. archive: `zip_rewrite(archive, dest, keep, base, h)` — сырой перенос записей по `keep` + `write_archive` с набором уже записанных папок (`have`).
2. repack: `rewrite_zip` — имена записей (`zip_names`: без шифрования, ссылок, повторов), проверки «занято / нет» для каждого `Change`, `apply` на пустом дереве, `keep` для Delete / Rename / Replace; иначе `None` → целиком. Общий хвост `finish`.
3. Тест: Stored-записи остаются Stored, папка не дублируется, Rename / Delete / Mkdir; конфликт — по-старому. Замер: F5 в zip 300 МБ — 0,9 с.
4. CLAUDE.md, ROADMAP, TESTING § 156; 0.70.0.
