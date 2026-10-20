# 164. Browse inside smb:/// — plan

Spec: `.claude/docs/specs/164-smb-browse.md`.

1. core `mount.rs`: `deeper(uri)` (smb with no share) + test; `browse(at, password, cancel)`:
   list, on «not mounted» mount and list again; `not_mounted(err)` + test.
2. app: `Message::ConnectBrowse(Option<String>)`, `Browsed(String, Result)`; `Dialog::Connect.at`
   (place listed); stale results dropped; app test for going deeper and a stale result.
3. dialogs: «›» button on `deeper` entries, caption with the place; strings en / ru.
4. tc-reference row for Ctrl+F, ROADMAP, TESTING, version 0.78.0.
