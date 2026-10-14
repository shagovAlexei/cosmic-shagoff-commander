# 78. Fonts

ROADMAP «Шрифты: свой шрифт и размер для разных частей».

- Settings → «Fonts»: «File list font» (installed families from libcosmic's font database, read
  when the settings open; first item = the interface's), «File list size», «Viewer (F3) size»
  (9–24). Config: `panel_font` ("" = default), `panel_size` (13), `lister_size` (14): the defaults
  keep the old look.
- Panels: names, ext, size, date, attributes in that font and size, icons scaled; the row height
  is the skin's scaled by size (`Config::row_h`), used everywhere the old `Skin::row_h` was (scroll,
  Brief grid, PgUp/PgDn), offsets rescaled when it changes.
- Viewer: size only — it stays monospaced, as hex, wrap and mouse selection count its columns;
  its row height and char width scale with the size (`Lister::row_h` / `mono_w`); a change while F3 is open keeps the top row and rewraps. Sizes are clamped to 9–24 when read. Brief column widths scale with the size.
- Not done: dialog lists (Ctrl+D, Alt+F1, history), menus (drawn by libcosmic), search in the
  font list, column widths following the size.
