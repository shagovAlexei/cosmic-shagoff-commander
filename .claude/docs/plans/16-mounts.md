# 16. Mounts Implementation Plan

**Goal:** Mount USB sticks / partitions from Alt+F1, connect to sftp/smb/ftp/dav with Ctrl+F, unmount with Ctrl+Shift+F; gvfs mounts get drive buttons.

**Architecture:** `core::mount` drives the `gio` CLI (no glib dependency); gvfs FUSE dirs make network paths plain paths. The app runs gio in `spawn_blocking` and reports in the status line.

**Spec:** `.claude/docs/specs/16-mounts.md`

## Global Constraints

- Tests never call the real `gio` (it would touch the user's session): the prompt loop is tested with a stand-in program.
- Every new string in `fl!`, en and ru. `just verify` green, commands chained with `&&`.

## Review Focus

1. A wrong password must not loop forever (gio re-asks endlessly) — `talk_wrong_password_stops`.
2. A host-key question must abort, not be answered with the password — `talk_question_aborts`.
3. Volumes for a closed / other-side drive list are dropped — app test.
4. Unmount of `/` or `~` is refused — app test.
5. A mount point with spaces (`%20` in the file URI) — `volumes_percent_decoded`.

### Task 1: core `mount.rs`
Parsing, labels, `answer`, `talk(cmd, password)` prompt loop, `mount_device`, `connect`, `unmount`; `refresh` adds `gvfs_drives`. Commit `feat(core): mount volumes and network shares through gio`.

### Task 2: app
`Action::Connect` (Ctrl+F), `Action::Disconnect` (Ctrl+Shift+F), `Dialog::Connect`, `ListItem.mount`, messages `Volumes`, `Mounted`, `Unmounted`; i18n. Commit `feat(app): mount, connect, unmount`.

### Task 3: docs
tc-reference (Ctrl+F, Ctrl+Shift+F, Alt+F1 note), ROADMAP, TESTING.md «Монтирование (16)», CLAUDE.md module list. Commit `docs: mounts (phase 16)`.
