# 173. Toolbar: drag buttons, working dir

ROADMAP «Настраиваемая панель инструментов, дальше: перетаскивание кнопок по панели, рабочий каталог кнопки».

## Drag (TC: Shift + drag on the bar)

- Shift + left press on a bar button holds it; while held, the button follows the pointer over the
  others (as Ctrl+D «Настроить…», 97); the release ends it. The order is saved into
  `config.toolbar` at once (the bar is the config).
- A Shift-press doesn't run the button (`Message::Tool` is ignored while Shift is held or a button
  is held).
- The hover / grab / "slot just vacated" logic of the hotlist moves into `core::reorder::Reorder`
  and both lists use it (separators are 10 px, buttons ~32: the same flicker as rows of other
  heights).
- Buttons keep the press, so press / release come from an `event::listen_with` subscription,
  on only while Shift is held or a button is held (classic skin).

## Working dir (TC «Рабочий каталог» / Start path)

- `ToolButton.dir` (serde default ""): the dialog has a field «Рабочий каталог».
- `cmdline::start_dir(dir, vars, home)`: empty → the panel's dir; `%P` / `%T` → the panel's /
  the other panel's dir (unquoted: it is a path, not a shell line); a leading `~` → home; relative
  → under the panel's dir. The program runs there (the `sh -c` cwd).
- Not done: F12's working dir (settings) — no one asked.
