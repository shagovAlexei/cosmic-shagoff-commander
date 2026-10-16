# 84. setuid / setgid / sticky in Properties

ROADMAP «Свойства, дальше: setuid/setgid/sticky флажками».

TC on Windows has no such bits; its Linux counterparts (Double Commander, `chmod`) show them as a
third row of boxes. We follow `ls` and DC.

- `format::perms` as `ls -l`: the x of owner / group / others shows `s` / `S` (setuid / setgid,
  with / without x) and `t` / `T` (sticky). The Attributes column and the status line show them.
- Alt+Enter: a «Особые» row under the rwx table with SUID / SGID / Sticky heads, the same boxes
  (`?` when the entries differ; recursive: every one of the 12 bits starts as `?`).
- `props::chmod` already masks 0o7777: only the toggled bits change. Recursive: setuid / setgid
  are put on the selected entries and the dirs inside, never on the files inside (setgid for a
  tree's dirs must not make every program in it set-group-id); sticky and taking any bit off
  reach the files too (checked by hand: off did not reach them at first). The dialog says so under
  the row while «И всё внутри каталогов» is on.
- Linux drops setgid without an error when the user is not in the file's group: after each
  change the mode is read back, and a difference is reported as a failure («mode is …, not …»).
- The `rwx` caption shows `?` in an x place when that place's special bit differs too.
- The rwx heads were cut («Выполнение»): cells 70 → 90 px.
