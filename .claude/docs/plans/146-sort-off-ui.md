# 146 — план

1. Замер (временный тест, release).
2. `Panel::set_sort_key`; `set_sort` через него.
3. `Action::Sort` при `entries > SORT_HERE_MAX` — ключ + `reload`.
4. Тест `a_big_list_is_sorted_by_a_background_read_keeping_cursor_and_marks`.
