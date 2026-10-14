# 77. Panel colours and the «Blue» palette

ROADMAP «Темы, дальше: свои цветовые палитры (TC-синий фон и т. п.)».

- `PanelColors` (was `MarkColors`; config key `panel`, was `marks`: colours set in 0.19 go back to the defaults once) gains `text` and `back`: the file
  list's text and background (TC Configuration → Colors «Text» / «Background»); empty = the theme's.
- Drawn by a container around the list (rows + the empty space under them): rows without a
  colour of their own inherit both; rule colours, marks and the cursor paint over.
- Dialog: a «Panel» button (Text / Background) above «Marked» / «Cursor»; «Add» / «Delete» become
  + / − icons beside ↑ ↓ (with tips), so the dialog keeps its height; footer: «Defaults», «Blue».
- «Blue» (`colors::blue_panel`): text `#ffffff` on `#000080` (cyan would hide the links' rule),
  marked `#ffff55`, cursor `#00c0c0` (light enough for black text); rules untouched.
- The column header takes the panel colours too.
- Not done: «Background 2» stripes, the header / path / tabs / F-keys in the palette's colours.
