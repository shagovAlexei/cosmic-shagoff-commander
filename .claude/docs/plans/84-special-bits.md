# 84. Special bits — plan

Spec: `../specs/84-special-bits.md`.

1. Test first: `perms(0o4755)` = `rwsr-xr-x`, `0o2644` = `rw-r-Sr--`, `0o1777` = `rwxrwxrwt`,
   `0o1770` = `rwxrwx--T`; implement in `format::perms`.
2. Core test: `chmod` sets and clears sticky / setuid.
3. Dialog: heads + «Особые» row with `bit(0o4000)`, `bit(0o2000)`, `bit(0o1000)`; recursive
   `mixed` = 0o7777; ftl en / ru.
4. headless: Alt+Enter at 1036x530.
