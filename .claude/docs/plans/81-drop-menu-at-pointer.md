# 81. Drop menu at the pointer — plan

Spec: `../specs/81-drop-menu-at-pointer.md`.

1. `route_event`: DnD Offer Enter / Motion → `Message::DropAt(x, y)` (window coordinates; a
   zone's `on_drop` is shifted by its scrollable); `App::drop_at: Option<Point>`.
2. `Message::Drop` (right drag): `Dialog::DropMenu { at: self.drop_at.take().unwrap_or_default(), .. }`; any other
   drop clears `drop_at`.
3. `dialogs::view`: `DropMenu` with `at` → `popover` over a full-window `mouse_area` (press →
   `DialogCancel`) with the menu card at `at` (the target dir in it).
4. Test: a right drag's drop after `DropAt` opens the menu with that point.
5. headless: the drop does not arrive in headless sway, so a test-only shot is not possible;
   TESTING § 81 by hand.
