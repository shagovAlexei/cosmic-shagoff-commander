# 88. Tab to the other pane — plan

Spec: `../specs/88-tab-to-other-pane.md`.

1. Test first: press a tab, pointer over its own pane / the other, release; with Ctrl.
2. `pane_hover`, `Message::TabDragAt(x)` from a subscription that exists only while `tab_drag`.
3. `ColDragEnd`: over the other pane → `act(from, TabMoveOther | TabCopyOther)`.
4. headless: a second tab from the Tabs menu (X11 mode), `drag` it onto the right list.
