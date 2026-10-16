# 85. Text size keys in Lister

ROADMAP «Lister, дальше: масштаб».

- In the viewer whatever key types `+` or `=` and `-` (by the character, not the key's place:
  German types ß where US has `-`), and
  Num+ / Num− (from the main table as SelectGroup / UnselectGroup, taken by `lister_action`), make
  the text one size bigger / smaller: `config::size_step` within `SIZES` (9..=24).
- It is the Settings value «Размер шрифта просмотрщика» (`Config.lister_size`), changed through
  `App::set(Setting::ListerSize)`: saved, and the open viewer rewrapped at the same top row by
  `apply_config`.
- Images: not zoomed yet (shown fitted), and the keys do nothing there (they must not change the
  text size unseen); TC's +/− zoom them — a later step. At 9 / 24 nothing is written.
