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

#[cfg(test)]
mod tests {
    use super::*;

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
