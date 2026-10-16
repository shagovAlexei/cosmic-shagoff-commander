# 100. Sync prefs — plan

Spec: `../specs/100-sync-prefs.md`.

1. `config::SyncPrefs` (serde default), `State.sync`, `sync::Kind` serializable.
2. `App.sync`: taken from the open dialog in `update`, read by `Action::SyncDirs`, saved with the state (test).
