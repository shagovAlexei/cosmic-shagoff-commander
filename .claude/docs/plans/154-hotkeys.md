# 154 — план

Спек: `specs/154-hotkeys.md`.

1. core `hotkey::Bind`: `parse`, `Display` — тесты.
2. keymap: `key_name` / `bind_of`, глобальный `OWN` + `set_own` / `own`, проверка первой в `action`; `CAPTURE`.
3. app `hotkeys.rs`: `commands` (меню + F1), `name`, `resolve`, `KeyEdit` + `view` — тесты; `Dialog::Keys`, `Message::KeyEdit`, `Action::ConfigureKeys`, пункт меню, `set_hotkeys` при OK, `apply_config` / `build` — `set_own`; `route_event` — захват.
4. Меню показывает свои клавиши (`menu::shown`). Строки en / ru, tc-reference, CLAUDE.md, ROADMAP, TESTING § 154; 0.68.0.
