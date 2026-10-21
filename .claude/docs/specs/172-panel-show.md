# 172. Which files a panel shows (Ctrl+F10 / Ctrl+F11 / Ctrl+F12)

TC View → «All files» (`cm_AllFiles`, Ctrl+F10), «Programs» (`cm_ExecutableFiles`, Ctrl+F11),
«User defined…» (`cm_UserSpec`, Ctrl+F12), missing until now.

- `core::panel::Show { All, Programs, Mask(String) }` on each panel (tab). Unlike the quick filter
  (Ctrl+S) it stays on in other dirs. Dirs are always shown, but those the mask's `|dir\` leaves out.
- Programs: files with an `x` bit (Linux has no `.exe`).
- Ctrl+F12: the Num+ mask dialog (same history), the mask in force offered; `*.*`, `*` or empty =
  all files. Rows it hides lose their marks (as the quick filter).
- The status line says «Shown: *.rs ·» / «Shown: programs ·» before the totals.
- View menu, F1. Not saved across restarts (TC keeps it in its ini).
