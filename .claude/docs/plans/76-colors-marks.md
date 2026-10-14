# 76. Colours: attributes, marked rows, cursor — plan

1. core `colors.rs`: `attrs()` + `Attrs::fits`, `Rule.attr`; `MarkColors` → `Marks`; `Scheme(.., Marks)` via `with_marks`. Tests `attributes`, `mark_colours`.
2. `config.rs`: `marks: MarkColors`. `app.rs`: scheme built `with_marks`, dialog gets / saves `marks`.
3. `view::cursor_style(.., marks)`; other callers pass `Default`.
4. `colorrules.rs`: `special: Option<Special>`, `ColorMsg::{Attr, Special}`, field «Attributes», «Marked» / «Cursor» buttons, «Defaults» in the footer. Test `marks_and_cursor_are_edited_apart_from_rules`.
5. F1 rows, i18n en + ru, tc-reference, ROADMAP, TESTING § 76; version 0.19.0; headless check (X11).
