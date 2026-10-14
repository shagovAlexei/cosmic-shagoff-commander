# 77. Panel colours and the «Blue» palette

ROADMAP «Темы, дальше: свои цветовые палитры (TC-синий фон и т. п.)».

- `PanelColors` (was `MarkColors`, config key `marks` kept) gains `text` and `back`: the file
  list's text and background (TC Configuration → Colors «Text» / «Background»); empty = the theme's.
- Drawn by a container around the list (rows + the empty space under them): rows without a
  colour of their own inherit both; rule colours, marks and the cursor paint over.
- Dialog: a «Panel» button (Text / Background) above «Marked» / «Cursor»; «Add» / «Delete» become
  + / − icons beside ↑ ↓ (with tips), so the dialog keeps its height; footer: «Defaults», «Blue».
- «Blue» (`colors::blue_panel`): text `#55ffff` on `#000080`, marked `#ffff55`, cursor `#00aaaa`;
  rules untouched.
- Not done: «Background 2» stripes, the header / path / tabs / F-keys in the palette's colours.
