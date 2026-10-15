# 83. Background 2 — plan

Spec: `../specs/83-background-2.md`.

1. Test first: `row_paint` with `odd` and `back2` (stripe, no stripe on even rows, marked without
   a marks background, the rule's and the cursor's win, nothing without `back2`).
2. `PanelColors.back2`, `PanelPaint.back2`, `row_paint(.., odd, ..)`.
3. `view::cursor_style(.., odd, ..)`: full list `i % 2`, Brief `(i - c * rows) % 2`, others false.
4. `colorrules`: `Special::Back2` (background only), the button, the test.
5. ftl `colors-back2`, help line; headless: stripes and the dialog at 1036x530.
