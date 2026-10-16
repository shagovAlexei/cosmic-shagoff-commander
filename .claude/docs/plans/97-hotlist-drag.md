# 97. Hotlist drag — plan

Spec: `../specs/97-hotlist-drag.md`.

1. `hotlist.rs`: `HotEdit.{hover, grab}`, `HotMsg::{Hover, Grab, Release, AddDirs}` (tests).
2. View: `mouse_area` per row / list, `clip::drop_zone` on the list; `App::subscription` listens to the left button while `Dialog::Hotlist` is up.
