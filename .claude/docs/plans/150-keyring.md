# 150 — план

Спек: `specs/150-keyring.md`.

1. core `keyring`: `gvfs_attrs(url)` (атрибуты как у gvfs), `keeps(url)` (схема), `label`, `save` (zbus, Secret Service). Тесты на атрибуты.
2. app: `Dialog::Connect.keep`, `Message::ConnectKeep` / `Kept`; на `Connected(Ok)` с флажком и паролем — `save` в фоне; ошибка — сообщением.
3. Флажок в окне, строки en / ru; tc-reference, CLAUDE.md, ROADMAP, TESTING § 150; 0.66.0.
