//! Thumbnails view (TC Ctrl+Shift+F1): cell sizes and the pictures, made off the UI thread.

use std::path::Path;

use cosmic::widget::image::Handle;

/// Side of the square a picture is shrunk into, px.
pub const SIZE: u32 = 96;
/// Cell width: the square and its padding.
pub const CELL_W: f32 = SIZE as f32 + 16.0;
/// Pictures made at once.
pub const BUSY_MAX: usize = 4;
/// Bigger files get their type's icon: decoding them would take long and a lot of memory.
const MAX_BYTES: u64 = 64 << 20;
/// The cache is dropped when it grows past this (~37 KB a picture).
pub const CACHE_MAX: usize = 2000;

/// Cell height: the square, the name line under it and padding.
pub fn cell_h(row_h: f32) -> f32 {
    SIZE as f32 + row_h + 8.0
}

/// Whether `name` gets a picture made (svg: not decodable here, it gets its icon).
pub fn wanted(name: &str, size: u64) -> bool {
    size <= MAX_BYTES
        && shagoff_core::lister::is_image(name)
        && !name.to_ascii_lowercase().ends_with(".svg")
}

/// `path`'s picture shrunk into `SIZE`², proportions kept; `None` if unreadable.
pub fn make(path: &Path) -> Option<Handle> {
    let bytes = std::fs::read(path).ok()?;
    let px = crate::lister::decode(&bytes)?
        .thumbnail(SIZE, SIZE)
        .into_rgba8();
    let (w, h) = px.dimensions();
    Some(Handle::from_rgba(w, h, px.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_the_square_and_keep_proportions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        image::RgbImage::new(2000, 1000).save(&path).unwrap();
        let Some(Handle::Rgba { width, height, .. }) = make(&path) else {
            panic!("no picture")
        };
        assert_eq!((width, height), (96, 48));
        assert!(make(&dir.path().join("none.png")).is_none());
        assert!(wanted("a.JPG", 10) && !wanted("a.svg", 10) && !wanted("a.txt", 10));
        assert!(!wanted("a.png", MAX_BYTES + 1));
    }
}
