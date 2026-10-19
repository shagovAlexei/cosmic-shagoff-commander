# 145 — Ctrl после ухода фокуса

Техдолг: Ctrl, отпущенный в другом окне, мог остаться «зажатым» (`App.mods`): `ModifiersChanged(empty)` при уходе фокуса зависит от winit / композитора.

- `route_event`: `window::Event::Unfocused` → `Message::Modifiers(empty)`. Не зависим от того, пришлёт ли композитор.
