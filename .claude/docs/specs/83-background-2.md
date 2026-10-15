# 83. Background 2

ROADMAP «Темы, дальше: «Фон 2» (полосы через строку)».

TC Configuration → Colors → «Background 2»: every second line of the file list gets a second
background colour.

- `PanelColors.back2` (`#RRGGBB`, empty = no stripes; `serde(default)`, old configs load);
  `PanelPaint.back2`.
- `colors::row_paint(.., odd, ..)`: an odd row with no background of its own (no rule background,
  not the cursor, not marked with a marks background) gets `back2`; its text stays the panel's
  (`Ink::Theme` / the marked text).
- Rows: the full list by entry index (stable while scrolling); Brief by the row within its column.
- The colour dialog: a «Фон 2» button under «Курсор», a background only like it
  (`Special::Back2`, `Special::bg_only`). «Синяя» has none (NC had no stripes).
