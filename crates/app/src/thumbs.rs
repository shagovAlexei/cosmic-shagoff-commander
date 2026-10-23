//! Thumbnails view (TC Ctrl+Shift+F1): cell sizes and the pictures, made off the UI thread.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use shagoff_core::thumbcache;
use shagoff_core::thumbnailer::{self, Registry};

use cosmic::widget::image::Handle;

/// Side of the square a picture is shrunk into, px: the default, and the range of
/// View → thumbnail zoom (`Config.thumb_size`, kept here as `size()`).
pub const SIZE: u32 = 96;
pub const SIZE_MIN: u32 = 48;
pub const SIZE_MAX: u32 = 256;
pub const SIZE_STEP: u32 = 32;
static CURRENT: AtomicU32 = AtomicU32::new(SIZE);

pub fn set_size(px: u32) {
    CURRENT.store(px.clamp(SIZE_MIN, SIZE_MAX), Ordering::Relaxed);
}

pub fn size() -> u32 {
    CURRENT.load(Ordering::Relaxed)
}

/// `px` one step bigger (`up`) or smaller, within the range.
pub fn zoomed(px: u32, up: bool) -> u32 {
    match up {
        true => (px + SIZE_STEP).min(SIZE_MAX),
        false => px.saturating_sub(SIZE_STEP).max(SIZE_MIN),
    }
}

/// Cell width: the square and its padding.
pub fn cell_w() -> f32 {
    size() as f32 + 16.0
}
/// Pictures made at once.
pub const BUSY_MAX: usize = 4;
/// A picture needing more to decode gets its type's icon (`BUSY_MAX` of them at once).
const MAX_ALLOC: u64 = 256 << 20;
/// The cache is dropped when it grows past this (~37 KB a picture).
pub const CACHE_MAX: usize = 2000;

/// Cell height: the square, the name line under it and padding.
pub fn cell_h(row_h: f32) -> f32 {
    cell_h_for(size(), row_h)
}

pub fn cell_h_for(px: u32, row_h: f32) -> f32 {
    px as f32 + row_h + 8.0
}

/// The size asked of a desktop thumbnailer (the spec's "normal"), shrunk to ours after; its
/// picture is read up to `EXTERNAL_MAX` bytes.
const EXTERNAL_SIZE: u32 = 128;

/// Asked of a thumbnailer: 128 (the spec's "normal"), more when the cells are bigger.
fn external_size() -> u32 {
    EXTERNAL_SIZE.max(size())
}
const EXTERNAL_MAX: u64 = 16 << 20;
/// A desktop thumbnailer gets this long before it is killed (a video read over the network…).
const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(10);

/// The desktop's thumbnailers, read once.
// ponytail: one installed while running is seen after a restart.
fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry::load(&shagoff_core::openwith::data_dirs()))
}

/// Whether `name` (`size` bytes) gets a picture: one we decode, or one a desktop thumbnailer
/// makes (PDF, video, SVG…). A picture too big for us gets its icon, not a thumbnailer.
pub fn wanted(name: &str, size: u64) -> bool {
    use shagoff_core::lister::{THUMB_MAX_BYTES, is_image};
    match is_image(name) && !name.to_ascii_lowercase().ends_with(".svg") {
        true => size <= THUMB_MAX_BYTES,
        false => registry().for_name(name).is_some(),
    }
}

/// `path`'s picture shrunk into `size()`², proportions kept; `None` if unreadable.
pub fn make(path: &Path, cache: bool) -> Option<Handle> {
    make_in(path, thumbcache::base_dir().filter(|_| cache).as_deref())
}

/// `make` with the shared cache (`thumbcache`) in `cache`: its picture of the file if it is the
/// file's current one, else made and put there; `None`: no cache.
fn make_in(path: &Path, cache: Option<&Path>) -> Option<Handle> {
    // Not a FIFO or a device named `*.png`: reading one would never end.
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy();
    if !wanted(&name, meta.len()) {
        return None;
    }
    let class = thumbcache::Class::for_px(size());
    let mtime = (meta.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let key = (cache.zip(mtime)).map(|(base, mtime)| {
        let uri = thumbcache::uri(path);
        (thumbcache::cache_file(base, class, &uri), uri, mtime)
    });
    let cached = key.as_ref().and_then(|(file, uri, mtime)| {
        let bytes = thumbcache::read(file, uri, *mtime, meta.len())?;
        crate::lister::decode(&bytes, MAX_ALLOC)
    });
    let img = match cached {
        Some(img) => img,
        None => {
            let bytes = match shagoff_core::lister::thumb_wanted(&name, meta.len()) {
                true => std::fs::read(path).ok()?,
                false => external(path, &name)?,
            };
            let made = crate::lister::decode(&bytes, MAX_ALLOC)?;
            // The cache keeps the class's size, shared with the others; ours is shrunk from it.
            // Shrunk only: the spec has no blurred upscales in the shared folder.
            let made = match made.width().max(made.height()) > class.px() {
                true => made.thumbnail(class.px(), class.px()),
                false => made,
            };
            if let Some((file, uri, mtime)) = &key {
                let rgba = made.to_rgba8();
                let (w, h) = rgba.dimensions();
                // ponytail: a failed write (read-only home, full disk) just means no cache.
                let _ = thumbcache::write(file, (w, h, rgba.as_raw()), uri, *mtime, meta.len());
            }
            made
        }
    };
    let px = img.thumbnail(size(), size()).into_rgba8();
    let (w, h) = px.dimensions();
    Some(Handle::from_rgba(w, h, px.into_raw()))
}

/// Drops `path`'s pictures from the shared cache (both classes).
pub fn forget(path: &Path) {
    let Some(base) = thumbcache::base_dir() else {
        return;
    };
    let uri = thumbcache::uri(path);
    for class in [thumbcache::Class::Normal, thumbcache::Class::Large] {
        let _ = std::fs::remove_file(thumbcache::cache_file(&base, class, &uri));
    }
}

/// The PNG the desktop's thumbnailer for `name` makes of `path`.
fn external(path: &Path, name: &str) -> Option<Vec<u8>> {
    static N: AtomicU64 = AtomicU64::new(0);
    static SANDBOX: OnceLock<bool> = OnceLock::new();
    let t = registry().for_name(name)?;
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    // In bwrap when it works and the program is the system's (GNOME does so too): a crafted file
    // meets a parser that sees nothing of the home. Else as cosmic-files: as is.
    let (args, out, scratch) =
        if thumbnailer::in_system(&t.exec) && *SANDBOX.get_or_init(thumbnailer::bwrap_works) {
            use std::os::unix::fs::DirBuilderExt;
            let dir = std::env::temp_dir().join(format!("shagoff-thumb.{pid}.{n}"));
            std::fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;
            let (args, out) = thumbnailer::sandboxed(&t.exec, path, &dir, external_size());
            (args, out, dir)
        } else {
            // GNOME's name in /tmp itself (not $TMPDIR): AppArmor lets evince-thumbnailer write only
            // there. Made first with O_EXCL: a link put in its place beforehand is not written through.
            let out = PathBuf::from(format!(
                "/tmp/.gnome_desktop_thumbnail.shagoff.{pid}.{n}.png"
            ));
            std::fs::File::create_new(&out).ok()?;
            let args = thumbnailer::argv(&t.exec, path, &out, external_size());
            (args, out.clone(), out)
        };
    let ok = thumbnailer::run(&args, EXTERNAL_TIMEOUT);
    let bytes = ok
        .then(|| thumbnailer::read_output(&out, EXTERNAL_MAX))
        .flatten();
    let _ = match scratch.is_dir() {
        true => std::fs::remove_dir_all(&scratch),
        false => std::fs::remove_file(&scratch),
    };
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_the_square_and_keep_proportions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        image::RgbImage::new(2000, 1000).save(&path).unwrap();
        let cache = tempfile::tempdir().unwrap();
        let Some(Handle::Rgba { width, height, .. }) = make_in(&path, Some(cache.path())) else {
            panic!("no picture")
        };
        assert_eq!((width, height), (96, 48));
        // Put in the shared cache at its 128 px, and read from there the next time.
        let uri = thumbcache::uri(&path);
        let file = thumbcache::cache_file(cache.path(), thumbcache::Class::Normal, &uri);
        assert!(file.is_file());
        std::fs::remove_file(&path).unwrap_or(());
        assert!(make_in(&path, Some(cache.path())).is_none()); // no file, no picture
        image::RgbImage::new(2000, 1000).save(&path).unwrap();
        assert!(make_in(&path, Some(cache.path())).is_some());
        assert!(make_in(&dir.path().join("none.png"), None).is_none());
        assert!(make_in(dir.path(), None).is_none()); // not a file
    }
}
