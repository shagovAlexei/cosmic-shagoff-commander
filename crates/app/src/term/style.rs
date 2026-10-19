//! What cosmic-term's `Config` gave its terminal: font weights, size and bright bold. Ours is
//! fixed but for the size (the panel font's), named as there so the adapted code reads the same.

use cosmic_text::{Metrics, Stretch, Weight};

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub font_size: f32,
    pub font_weight: u16,
    pub dim_font_weight: u16,
    pub bold_font_weight: u16,
    pub use_bright_bold: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            font_weight: Weight::NORMAL.0,
            dim_font_weight: Weight::NORMAL.0,
            bold_font_weight: Weight::BOLD.0,
            use_bright_bold: false,
        }
    }
}

impl Style {
    pub fn typed_font_stretch(&self) -> Stretch {
        Stretch::Normal
    }

    /// Line height 1.4 × the size, as cosmic-term; `zoom_adj` steps of 1 px.
    pub fn metrics(&self, zoom_adj: i8) -> Metrics {
        let size = (self.font_size + f32::from(zoom_adj)).max(1.0);
        Metrics::new(size, (size * 1.4).ceil())
    }
}
