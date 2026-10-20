# 169. Folder tree (Alt+F10)

TC «Change directory via tree» (Alt+F10). The tree in a panel (Ctrl+F8) is a later step.

- A list dialog of the dirs as an outline from `/`: opened down to the active panel's dir (inside
  an archive: the archive's dir), the cursor on it; the way there is shown even through hidden
  dirs (`tree::subdirs_toward`), other hidden dirs as the panel shows them.
- Keys as TC's tree: ↑ ↓ PgUp PgDn Home End; → opens a dir (its subdirs read off the UI thread,
  `tree_read` → `Message::TreeFilled`), again → steps in; ← closes an open one, else goes to the
  parent (`Tree::step_in` / `step_out`); a letter — the next dir with it; Enter / «Перейти» — that
  dir in the active panel; a click picks a row, a click on the picked one opens / closes it
  (`toggle`). Esc closes. Reads carry the dialog's id (a late one for an earlier dialog is
  dropped) and never refill an open dir. Names are kept as read (`OsString`).
- Core `tree`: `Tree` (flat rows, `fill` / `close` / `parent` / `select`), `chain`, `subdirs`.
