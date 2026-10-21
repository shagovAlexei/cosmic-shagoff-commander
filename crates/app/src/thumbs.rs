//! Thumbnails view (TC Ctrl+Shift+F1): cell sizes and the pictures, made off the UI thread.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use shagoff_core::thumbnailer::{Registry, argv};

use cosmic::widget::image::Handle;

/// Side of the square a picture is shrunk into, px.
pub const SIZE: u32 = 96;
/// Cell width: the square and its padding.
pub const CELL_W: f32 = SIZE as f32 + 16.0;
/// Pictures made at once.
pub const BUSY_MAX: usize = 4;
/// A picture needing more to decode gets its type's icon (`BUSY_MAX` of them at once).
const MAX_ALLOC: u64 = 256 << 20;
/// The cache is dropped when it grows past this (~37 KB a picture).
pub const CACHE_MAX: usize = 2000;

/// Cell height: the square, the name line under it and padding.
pub fn cell_h(row_h: f32) -> f32 {
    SIZE as f32 + row_h + 8.0
}

/// A desktop thumbnailer gets this long before it is killed (a video read over the network…).
const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(10);

/// The desktop's thumbnailers, read once.
// ponytail: one installed while running is seen after a restart.
fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry::load(&shagoff_core::openwith::data_dirs()))
}

/// Whether `name` (`size` bytes) gets a picture: one we decode, or one a desktop thumbnailer
/// makes (PDF, video, SVG…).
pub fn wanted(name: &str, size: u64) -> bool {
    shagoff_core::lister::thumb_wanted(name, size) || registry().for_name(name).is_some()
}

/// `path`'s picture shrunk into `SIZE`², proportions kept; `None` if unreadable.
pub fn make(path: &Path) -> Option<Handle> {
    // Not a FIFO or a device named `*.png`: reading one would never end.
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy();
    let bytes = match shagoff_core::lister::thumb_wanted(&name, meta.len()) {
        true => std::fs::read(path).ok()?,
        false => external(path, &name)?,
    };
    let px = crate::lister::decode(&bytes, MAX_ALLOC)?
        .thumbnail(SIZE, SIZE)
        .into_rgba8();
    let (w, h) = px.dimensions();
    Some(Handle::from_rgba(w, h, px.into_raw()))
}

/// The PNG the desktop's thumbnailer for `name` makes of `path`.
fn external(path: &Path, name: &str) -> Option<Vec<u8>> {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = registry().for_name(name)?;
    // GNOME's name in /tmp itself (not $TMPDIR): AppArmor lets evince-thumbnailer write only
    // there. Made first with O_EXCL: a link put in its place beforehand is not written through.
    let out = std::path::PathBuf::from(format!(
        "/tmp/.gnome_desktop_thumbnail.shagoff.{}.{}.png",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create_new(&out).ok()?;
    // The size the spec calls "normal"; shrunk to ours after.
    let args = argv(&t.exec, path, &out, 128);
    let (prog, rest) = args.split_first()?;
    let child = std::process::Command::new(prog)
        .args(rest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        let _ = std::fs::remove_file(&out);
        return None;
    };
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if start.elapsed() < EXTERNAL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = ok.then(|| std::fs::read(&out).ok()).flatten();
    let _ = std::fs::remove_file(&out);
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
        let Some(Handle::Rgba { width, height, .. }) = make(&path) else {
            panic!("no picture")
        };
        assert_eq!((width, height), (96, 48));
        assert!(make(&dir.path().join("none.png")).is_none());
        assert!(make(dir.path()).is_none()); // not a file
    }
}
