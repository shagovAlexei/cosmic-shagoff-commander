# 89. Attributes in the file search

ROADMAP «Поиск, дальше: фильтр атрибутов».

TC Alt+F7 → «Advanced»: Windows attributes (archive, read-only, hidden, system…) as three-state
boxes. Ours follow the colour rules' field (phase 76) instead of new boxes.

- `colors::Attrs` gains `x` (a file with a run bit; a dir's x is not that) and `fits_mode(name,
  is_link, is_dir, mode)`, shared by the colour rules and the search. A link's own mode counts
  (never read-only, special or a program).
- `search::Query.attrs`: every asked letter must hold, by `DirEntry::metadata` (not following
  links). `h` makes the search look into hidden names even when the panel hides them. With
  attributes asked, archive members are not listed (no modes there).
- The dialog: «Атрибуты (h r s x)» after «Не старше, дней», remembered in `FindPrefs.attrs`; a
  letter that is none of them stops the search with a message.
