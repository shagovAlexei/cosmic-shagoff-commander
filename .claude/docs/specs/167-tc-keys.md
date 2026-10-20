# 167. TC keys: Num /, Alt+Num ±, Shift+F10, Ctrl+← / →, Ctrl+L

Gaps found comparing with TC's keyboard list (after 166).

- **Num /** — restore the selection (TC `cm_RestoreSelection`): the marks as they were before the
  last time they went in bulk — unmarked after an operation (`Panel::unmark`) or Ctrl+Num− /
  «снять всё» (`mark_all(false)`). Kept per panel for its dir; names no longer listed are skipped.
- **Alt+Num + / Alt+Num −** — mark / unmark every file with the extension of the one under the
  cursor (`cm_SelectCurrentExtension`); files without one: the ones without one. On a dir or `..`:
  nothing.
- **Shift+F10 / the Menu key** — the row's context menu from the keyboard. libcosmic opens its
  context menu only on a right-button release, so it is a list dialog with the same items
  (`menu::context_items`, dividers as separators), driven like the Ctrl+D menu (↑ ↓ Enter, letters).
- **Ctrl+← / Ctrl+→** — open in the left / right panel what is under the cursor (TC): a dir, `..`
  (the parent), an archive; on a file — the active panel's dir (`Panel::drop_target` /
  `drop_dir`).
- **Ctrl+L** — occupied space of the targets (TC `cm_CountDirContent`-like «Занятое место»):
  counted in the background (`props::usage`), the result in the status line: bytes, files, dirs.
