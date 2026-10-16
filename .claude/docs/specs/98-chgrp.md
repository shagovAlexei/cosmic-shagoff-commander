# 98. Changing the group

ROADMAP «Свойства, дальше: смена владельца/группы (нужен root или своя группа)». Without root a
file's owner may give it only one of his own groups; the owner can't be changed at all.

- Alt+Enter: «Группа» drop-down — the user's groups (`owners::my_groups`: getgroups + egid) and
  the files' own; the files' common group shown (none if they differ). Picked → applied on
  «Применить», with «рекурсивно» to everything inside.
- `props::chgrp`: `lchown` (links themselves, never their targets), entries already in the group
  left alone; failures reported as chmod's. The kernel drops setuid / setgid on a non-root chown:
  `chgrp` puts them back (same owner). The group goes first, the toggled bits after it.
- Picking the files' own group again does nothing (`Props::new_group`). Files the user does not own
  (and not root): the group is shown as text, no drop-down.
- Not done: changing the owner (root only — a pkexec helper later, if asked for).
