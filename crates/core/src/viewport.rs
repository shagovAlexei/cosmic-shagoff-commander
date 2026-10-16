//! Scroll math for the virtualized file list (fixed row height).

use std::ops::Range;

/// Extra rows rendered above and below the viewport.
pub const OVERSCAN: usize = 2;

/// Rows to render for a viewport at `offset` px with `height` px, plus `OVERSCAN` on each side.
pub fn visible_range(len: usize, row_h: f32, offset: f32, height: f32) -> Range<usize> {
    if len == 0 || row_h <= 0.0 {
        return 0..0;
    }
    let first = ((offset.max(0.0) / row_h) as usize).min(len);
    let count = (height.max(0.0) / row_h).ceil() as usize + 1;
    first.saturating_sub(OVERSCAN)..(first + count + OVERSCAN).min(len)
}

/// Offset that brings row `cursor` fully into view, or `None` if it already is.
pub fn scroll_to_cursor(cursor: usize, row_h: f32, offset: f32, height: f32) -> Option<f32> {
    let top = cursor as f32 * row_h;
    let bottom = top + row_h;
    if top < offset {
        Some(top)
    } else if bottom > offset + height {
        Some((bottom - height).max(0.0))
    } else {
        None
    }
}

/// PgUp/PgDn step: full rows on screen minus one, at least 1.
pub fn page_rows(row_h: f32, height: f32) -> usize {
    ((height.max(0.0) / row_h) as usize)
        .saturating_sub(1)
        .max(1)
}

/// Brief view: narrowest column, px; the pane width is shared by as many as fit.
/// Full view: a column's width bounds when dragged by its header edge.
pub const COL_MIN_W: f32 = 30.0;
pub const COL_MAX_W: f32 = 600.0;

/// The panes' divider dragged to window x `x`: the left pane's share of `total` (both panes'
/// widths, the gap left out), its left edge at `left`. Kept within 15 %…85 %, as TC keeps both
/// panels usable. A share outside (an old or hand-edited state) means half.
pub fn split_ratio(left: f32, total: f32, x: f32) -> f32 {
    if total <= 0.0 {
        return 0.5;
    }
    ((x - left) / total).clamp(0.15, 0.85)
}

/// The narrowest Name column: past it the fixed columns on the right are left out, last first.
pub const NAME_MIN: f32 = 100.0;

/// How many of the fixed columns (Ext, Size, Date, Attributes) fit beside a Name of at least
/// `NAME_MIN` in a list `list_w` wide (rows: 6 px padding each side, 6 px between cells).
/// 0 before the first layout shows them all.
pub fn visible_cols(list_w: f32, cols: [f32; 4]) -> usize {
    if list_w <= 0.0 {
        return 4;
    }
    (0..=4)
        .rev()
        .find(|&n| list_w - 12.0 - cols[..n].iter().map(|w| w + 6.0).sum::<f32>() >= NAME_MIN)
        .unwrap_or(0)
}

/// The share as stored: anything outside 15 %…85 % (0 = never set) is half.
pub fn split_or_half(r: f32) -> f32 {
    if (0.15..=0.85).contains(&r) { r } else { 0.5 }
}

/// Header edge `edge` (1 = right of Name … 4 = left of Attributes) dragged from `start_x` to
/// `x`: which of the four fixed columns changes, and its new width from `start_w`. As in TC an
/// edge sizes the column on its left; Name takes the rest, so its own edge sizes Ext inversely.
pub fn drag_col(edge: usize, start_w: f32, start_x: f32, x: f32) -> (usize, f32) {
    let dx = x - start_x;
    let (col, w) = match edge {
        1 => (0, start_w - dx),
        e => (e - 2, start_w + dx),
    };
    (col, w.clamp(COL_MIN_W, COL_MAX_W))
}

/// The column `drag_col` changes for `edge`.
pub fn drag_target(edge: usize) -> usize {
    edge.saturating_sub(2)
}

/// Brief column width bounds: a column fits the longest name, as in TC, within these.
pub const BRIEF_MIN_W: f32 = 120.0;
pub const BRIEF_MAX_W: f32 = 480.0;

/// Brief column width for names up to `longest` chars (icon and padding included), at `scale`
/// times the default text size.
pub fn brief_col_w(longest: usize, scale: f32) -> f32 {
    ((longest as f32 * 7.2 + 44.0) * scale).clamp(BRIEF_MIN_W, BRIEF_MAX_W * scale.max(1.0))
}

/// Brief view: rows per column that fit `height`, at least 1.
pub fn brief_rows(row_h: f32, height: f32) -> usize {
    ((height.max(0.0) / row_h) as usize).max(1)
}

/// Brief view: columns of `col_w` that fit `width`, at least 1.
pub fn brief_cols(width: f32, col_w: f32) -> usize {
    ((width.max(0.0) / col_w) as usize).max(1)
}

/// Brief view: first visible column keeping the cursor's column on screen, moved as little as
/// possible from `first`, never past the point where the last column ends the screen.
pub fn brief_first_col(len: usize, cursor: usize, rows: usize, cols: usize, first: usize) -> usize {
    let col = cursor / rows;
    let last_first = len.div_ceil(rows).saturating_sub(cols);
    let first = if col < first {
        col
    } else if col >= first + cols {
        col + 1 - cols
    } else {
        first
    };
    first.min(last_first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_follows_the_pointer_within_bounds() {
        assert_eq!(split_ratio(8.0, 1000.0, 308.0), 0.3);
        assert_eq!(split_ratio(0.0, 1000.0, 10.0), 0.15);
        assert_eq!(split_ratio(0.0, 1000.0, 990.0), 0.85);
        assert_eq!(split_ratio(0.0, 0.0, 10.0), 0.5); // before the first layout
        assert_eq!(
            (split_or_half(0.0), split_or_half(0.3), split_or_half(0.9)),
            (0.5, 0.3, 0.5)
        );
    }

    #[test]
    fn narrow_lists_drop_columns_from_the_right() {
        let cols = [50.0, 80.0, 120.0, 80.0]; // all four + gaps: 354
        assert_eq!(visible_cols(0.0, cols), 4);
        assert_eq!(visible_cols(12.0 + 354.0 + 100.0, cols), 4);
        assert_eq!(visible_cols(12.0 + 354.0 + 99.0, cols), 3); // attributes go first
        assert_eq!(visible_cols(300.0, cols), 2);
        assert_eq!(visible_cols(50.0, cols), 0);
    }

    #[test]
    fn brief_rows_and_cols_at_least_one() {
        assert_eq!(brief_rows(20.0, 400.0), 20);
        assert_eq!(brief_rows(20.0, 5.0), 1);
        assert_eq!(brief_cols(450.0, 200.0), 2);
        assert_eq!(brief_cols(0.0, 200.0), 1);
    }

    #[test]
    fn dragging_a_column_edge() {
        // Name's edge 20 px right: Name wider, so Ext narrower.
        assert_eq!(drag_col(1, 60.0, 500.0, 520.0), (0, 40.0));
        // Size|Date edge 20 px right: Size (on its left) wider.
        assert_eq!(drag_col(3, 90.0, 500.0, 520.0), (1, 110.0));
        assert_eq!(drag_col(4, 130.0, 500.0, -900.0), (2, COL_MIN_W));
        assert_eq!(drag_col(2, 60.0, 0.0, 9000.0), (0, COL_MAX_W));
        assert_eq!((drag_target(1), drag_target(2), drag_target(4)), (0, 0, 2));
    }

    #[test]
    fn brief_columns_fit_the_longest_name_within_bounds() {
        assert_eq!(brief_col_w(3, 1.0), BRIEF_MIN_W);
        assert!((brief_col_w(40, 1.0) - 332.0).abs() < 1.0);
        assert_eq!(brief_col_w(500, 1.0), BRIEF_MAX_W);
        assert!((brief_col_w(40, 2.0) - 664.0).abs() < 1.0); // bigger text, wider columns
        assert_eq!(brief_col_w(500, 2.0), 2.0 * BRIEF_MAX_W);
    }

    #[test]
    fn brief_first_col_follows_cursor() {
        // 10 rows, 3 columns on screen, 100 entries = 10 columns
        assert_eq!(brief_first_col(100, 15, 10, 3, 0), 0); // column 1: on screen
        assert_eq!(brief_first_col(100, 35, 10, 3, 0), 1); // column 3: shift by one
        assert_eq!(brief_first_col(100, 5, 10, 3, 4), 0); // column 0: left of screen
        assert_eq!(brief_first_col(100, 99, 10, 3, 0), 7);
        // grown window: no empty columns left at the end
        assert_eq!(brief_first_col(100, 99, 10, 5, 7), 5);
        assert_eq!(brief_first_col(0, 0, 10, 3, 2), 0);
    }

    #[test]
    fn visible_range_empty() {
        assert_eq!(visible_range(0, 20.0, 0.0, 400.0), 0..0);
    }

    #[test]
    fn visible_range_top_and_middle() {
        // 400 px / 20 px = 20 rows (+1 partial) + overscan
        assert_eq!(visible_range(10_000, 20.0, 0.0, 400.0), 0..23);
        // offset 1000 px → first visible row 50
        assert_eq!(visible_range(10_000, 20.0, 1000.0, 400.0), 48..73);
    }

    #[test]
    fn visible_range_clamped_to_len() {
        assert_eq!(visible_range(5, 20.0, 0.0, 400.0), 0..5);
        assert_eq!(visible_range(30, 20.0, 9999.0, 400.0), 28..30);
    }

    #[test]
    fn scroll_to_cursor_only_when_needed() {
        // rows 0..20 visible at offset 0
        assert_eq!(scroll_to_cursor(5, 20.0, 0.0, 400.0), None);
        // row 20 is just below → scroll so its bottom touches the viewport bottom
        assert_eq!(scroll_to_cursor(20, 20.0, 0.0, 400.0), Some(20.0));
        // row 3 is above the offset → scroll to its top
        assert_eq!(scroll_to_cursor(3, 20.0, 200.0, 400.0), Some(60.0));
        // cursor 0 after a dir change with an old offset
        assert_eq!(scroll_to_cursor(0, 20.0, 500.0, 400.0), Some(0.0));
    }

    #[test]
    fn page_rows_at_least_one() {
        assert_eq!(page_rows(20.0, 400.0), 19);
        assert_eq!(page_rows(20.0, 10.0), 1);
        assert_eq!(page_rows(20.0, 0.0), 1);
    }
}
