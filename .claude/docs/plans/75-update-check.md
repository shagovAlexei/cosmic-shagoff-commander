# 75. Check for updates — plan

1. `crates/core/src/update.rs`: `tag(json)`, `newer(current, tag)` test-first; `latest()` runs curl.
2. `app.rs`: `update_check: Option<Option<Result<String, String>>>`, `CheckUpdate` → `spawn_blocking(latest)` → `UpdateChecked`.
3. `drawer.rs`: `update_check()` view between donate and the about widget; strings `update-*` en + ru.
4. `justfile`: `curl` in the .deb Depends. Version 0.18.0.
5. Headless (X11 for the menu): About → button → «latest». TESTING § 75, README en + ru, ROADMAP.
