# 83. Background 2

ROADMAP «Темы, дальше: «Фон 2» (полосы через строку)».

TC Configuration → Colors → «Background 2»: every second line of the file list gets a second
background colour.

- `PanelColors.back2` (`#RRGGBB`, empty = no stripes; `serde(default)`, old configs load);
  `PanelPaint.back2`.
- `colors::row_paint(.., odd, ..)`: an odd row with no background of its own (no rule background,
  not the cursor, not marked with a marks background) gets `back2`; its text: the panel's if set,
  else black or white by `back2` (as on a rule's background); a marked row keeps the marked text.
- Brief stripes restart in every column (rows line up across columns), so an entry's stripe can
  differ from the column view's. The inactive pane's faint cursor is drawn over the list
  background, not over the stripe.
- Rows: the full list by entry index (stable while scrolling); Brief by the row within its column.
- The colour dialog: a «Фон 2» button under «Курсор», a background only like it
  (`Special::Back2`, `Special::bg_only`). «Синяя» has none (NC had no stripes).
