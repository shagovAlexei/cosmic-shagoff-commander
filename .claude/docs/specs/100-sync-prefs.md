# 100. Sync: remembered options

ROADMAP «Синхронизация, дальше: сохранение настроек». Ctrl+Shift+S opened with the same defaults
every time.

- `State.sync: SyncPrefs` (as `FindPrefs`): recursive, content, ignore date, mirror, mask and the
  hidden row kinds; taken when «Сравнить» starts, the dialog opens with them. Hidden files still
  follow the panel. Older state files: defaults (recursive, equal rows hidden).
