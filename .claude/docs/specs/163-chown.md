# 163. Changing the owner

ROADMAP «Свойства, дальше: смена владельца (нужен root)»; spec 98 left it for «a pkexec helper later».

- Alt+Enter: «Владелец» drop-down — every user of `/etc/passwd` (and the files' own uid), the
  files' common owner shown (none if they differ). «Группа» now lists every group of
  `/etc/group`, not only the user's.
- On «Применить»: a new owner, a group that is not one of the user's (`owners::my_groups`), or
  files the user does not own (`Props::foreign`), while not root, go through
  `pkexec shagoff-commander --props-apply uid gid set clear recursive -- paths`
  (`props::helper_args` / `run_helper`, caught at the top of `main` before any window): the
  program itself as root does what OK does without root, `props::apply` = `props::chown`
  (`lchown`, links themselves, setuid / setgid put back) then `props::chmod` — one password, one
  code path, the same rules for links and recursion. Its failures come back on stderr, line by
  line; polkit's refusal or a missing pkexec is shown the same way. Root runs `apply` itself.
- `props::chgrp` became `props::chown(uid, gid)`.
- In an archive: nothing (as 116).
- Not done: a list of «human» users only (every user is offered: www-data and such are real
  targets).
