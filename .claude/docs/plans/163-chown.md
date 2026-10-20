# Plan 163: changing the owner

1. core `owners.rs`: `Owners::users(&uids)` (as `groups`), `all_uids()` / `all_gids()`; test.
2. core `props.rs`: `chown_args(paths, uid, gid, recursive)` (argv, test) and
   `chown_with(prefix, …) -> Result<(), String>` (test: `chown` to the user's own uid, no prefix).
3. app `dialogs::Props`: `uids`, `user_names`, `user`, `user0`, `new_user()`; `can_chgrp` gone
   (the drop-downs are always there outside an archive); view: «Владелец» drop-down above «Группа».
4. app: `Message::PropsUser`; `props_dialog` fills both lists; `submit_dialog`: root path
   (`pkexec` unless euid 0) after the bits, the own-group path as before.
5. i18n en/ru (none new expected but the error), tc-reference row, TESTING.md, ROADMAP, version 0.77.0.
