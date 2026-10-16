# 97. Hotlist drag — plan

Spec: `../specs/97-hotlist-drag.md`.

1. `hotlist.rs`: `HotEdit.{hover, grab, vacated}`, `HotMsg::{Hover, Left, Grab, Release}` (tests).
2. View: `mouse_area` per row, scrollbar with spacing; `App::subscription` listens to the left button and the wheel while `Dialog::Hotlist` is up.
