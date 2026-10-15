# 81. Drop menu at the pointer — plan

Spec: `../specs/81-drop-menu-at-pointer.md`.

1. `clip::drop_zone`: `.on_drop(|x, y| Message::DropAt(x, y))`; `App::drop_at: Option<Point>`.
2. `Message::Drop` (right drag): `Dialog::DropMenu { at: self.drop_at.take(), .. }`; any other
   drop clears `drop_at`.
3. `dialogs::view`: `DropMenu` with `at` → `popover` over a full-window `mouse_area` (press →
   `DialogCancel`) with the menu card at `at`; without `at` → the old dialog.
4. Test: a right drag's drop after `DropAt` opens the menu with that point.
5. headless: the drop does not arrive in headless sway, so a test-only shot is not possible;
   TESTING § 81 by hand.
