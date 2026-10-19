# 133 — план

1. `Item::Add`: `cwd.to_str()` нет — ошибка `hotlist-not-utf8`.
2. `save_hotlist`: ошибка — `config-not-saved` в строку состояния.
3. Тест `regression_hotlist_add_of_a_non_utf8_dir_says_why` (красный на старом коде); строки en + ru.
