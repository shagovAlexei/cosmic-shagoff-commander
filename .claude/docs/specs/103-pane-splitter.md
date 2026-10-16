# 103. Divider between the panes

The user: the panes' widths could not be changed. TC: the bar between the panels is dragged; its
place is kept.

- The gap between the panes is the divider: ↔ pointer, pressed → the share follows the pointer
  (`viewport::split_ratio`, 15 %…85 %), double click → half and half. Kept in `State.split`
  (0 = never moved: half). The footer's totals follow it.
- A narrow pane stays readable: Name keeps ≥ 100 px, the fixed columns that don't fit go,
  rightmost first (`viewport::visible_cols`), in the rows and the header alike; the drive buttons
  (Modern) scroll sideways on one line, the free space is cut with «…» instead of wrapping.
