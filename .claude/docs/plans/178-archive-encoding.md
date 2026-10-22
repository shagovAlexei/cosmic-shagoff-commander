# 178. План: кодировка имён в архиве

1. core: `NameEnc`, `set_name_encoding` / `name_encoding`, `zip_name` + `looks_cp866`, применить в `read_zip`, `files`, `read_index`; ключ кэша `INDEX` с кодировкой. Тесты: чистая функция, список и распаковка zip в cp866.
2. app: `Action::ArchiveEncoding`, `ListKind::NameEncoding`, меню «Файлы», кнопка `cm_ChangeArchiveEncoding`, справка, строки en/ru; тест.
3. headless: zip с cp866 — «Авто» по-русски при LANG=en_US, выбор cp437 / обратно.
4. Документы, версия 0.92.0.
