# 176. Edit the path line (TC `cm_EditPath`)

ROADMAP «Сверка с командами TC», 176.

- TC: a click on the path line where no part of the path is, or Shift+F6 on `[..]` with nothing
  marked, makes the path editable; `cm_EditPath` has no key of its own (users bind one).
- Ours: the same, plus Commands → «Правка пути» and `Action::EditPath` (assignable in Hotkeys).
  F2 is our Shift+F6, so it works too.
- The line becomes a text field with the shown path (a nested archive's `outer.zip/…`), all
  selected. Enter: `cmdline::resolve` (as `cd`: quotes, `~`, relative to the panel, `..`), then
  `App::real`; then off the UI thread (a dead mount would hang a stat) `file_in_its_dir`: a file —
  on disk or in an archive, by its dir's listing — → its dir, the cursor on it; an archive or a
  dir → there (`load`, which reports a missing one). Esc or the field losing the focus (TC) leaves
  it. Not under the viewer in the panels' place.
