# 114 — Alt+L «В панель»: план

Спек: `../specs/114-find-feed-key.md`.

1. `route_event`: Alt + физическая L при любом статусе → `Message::FeedKey(Option<Action>)` (то, что ушло бы иначе: `keymap::action` при `Ignored`). Тест.
2. `update`: открыт поиск → `find_feed`, иначе — `Key(action)`. Тест.
3. Подсказка кнопки / справка, tc-reference, TESTING § 114, ROADMAP, 0.57.0 (после 113 — 0.56.0; если 113 смержен раньше).
