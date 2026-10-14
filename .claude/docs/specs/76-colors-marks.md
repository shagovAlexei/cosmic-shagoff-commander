# 76. Colours: attributes, marked rows, cursor

The rest of ROADMAP «Цвета, дальше».

- **Attributes** (`Rule.attr`): letters `h` hidden (name starts with `.`), `r` read-only (no owner
  `w`), `s` setuid / setgid / sticky; all given must hold; spaces / commas ignored, any case;
  another letter → the rule is invalid (⚠). Links never match `r` / `s` (their own mode is 0o777).
  TC does attributes through a search template; we keep the three that matter on Linux.
- **Marked / Cursor** (TC «Mark color», «Cursor color»): `Config.marks: MarkColors { color, bg,
  cursor }`, `#RRGGBB`, empty = the theme's. Edited in the same «Colors by type» dialog: buttons
  «Marked» (Text / Background) and «Cursor» (Background only) under the rule buttons; while one is
  lit the colour field and the palette edit it, no rule is selected; empty is named «None».
- Drawing (`view::cursor_style`): cursor in the active pane → `cursor` bg (text black / white by
  brightness); inactive → the same at 35 %; cursor on a marked row → `color` as bg (as the theme's
  red does); marked rows → `bg`, text `color` (else black / white on `bg`, else the theme's red).
- «Defaults» moves to the dialog's footer (tertiary action) so the dialog still fits 530 px with
  the palette open; it also clears the marks' colours.
