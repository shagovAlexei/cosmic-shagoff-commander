# 100. Sync: remembered options

ROADMAP «Синхронизация, дальше: сохранение настроек». Ctrl+Shift+S opened with the same defaults
every time.

- `State.sync: SyncPrefs` (as `FindPrefs`): recursive, content, ignore date, mask and the hidden
  row kinds, taken from the open dialog after every message (so as on screen when it closes); the
  dialog opens with them. Hidden files still follow the panel. Older state files: defaults
  (recursive, equal rows hidden).
- «Зеркало» is not kept: on, it plans the right's extra files for the trash, so it is turned on
  each time.
