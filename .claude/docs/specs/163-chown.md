# 163. Changing the owner

ROADMAP «Свойства, дальше: смена владельца (нужен root)»; spec 98 left it for «a pkexec helper later».

- Alt+Enter: «Владелец» drop-down — every user of `/etc/passwd` (and the files' own uid), the
  files' common owner shown (none if they differ). «Группа» now lists every group of
  `/etc/group`, not only the user's.
- On «Применить»: a new owner, or a group that is not one of the user's (`owners::my_groups`)
  while not root, goes through `pkexec chown -h [-R] +uid:+gid -- paths` (numbers with `+`, so a
  user named like a number can't be taken; `-h`: links themselves; GNU `-R` doesn't follow
  links). polkit asks for the password; its refusal or a missing pkexec is shown as an error.
  Root runs `chown` itself, without pkexec. The user's own groups keep the old way
  (`props::chgrp`, no password).
- Order with root: the toggled bits first (the user still owns the files), the chown after. The
  kernel drops setuid / setgid on a chown, as the `chown` command does; not put back.
- In an archive: nothing (as 116).
- Not done: a list of «human» users only; the owner of files of the archive.
