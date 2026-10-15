# 84. setuid / setgid / sticky in Properties

ROADMAP «Свойства, дальше: setuid/setgid/sticky флажками».

TC on Windows has no such bits; its Linux counterparts (Double Commander, `chmod`) show them as a
third row of boxes. We follow `ls` and DC.

- `format::perms` as `ls -l`: the x of owner / group / others shows `s` / `S` (setuid / setgid,
  with / without x) and `t` / `T` (sticky). The Attributes column and the status line show them.
- Alt+Enter: a «Особые» row under the rwx table with SUID / SGID / Sticky heads, the same boxes
  (`?` when the entries differ; recursive: every one of the 12 bits starts as `?`).
- `props::chmod` already masks 0o7777: only the toggled bits change. Setting setgid on a file of a
  group one is not in is refused by the kernel and reported as any chmod error.
- The rwx heads were cut («Выполнение»): cells 70 → 90 px.
