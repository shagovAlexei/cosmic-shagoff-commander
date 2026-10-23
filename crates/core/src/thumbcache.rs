//! The thumbnail cache of the freedesktop spec, shared with Files, GNOME and the rest:
//! `~/.cache/thumbnails/{normal,large}/<md5 of the file's URI>.png` (128 / 256 px), the PNG telling
//! which file and which mtime it was made from (`Thumb::URI`, `Thumb::MTime`, `Thumb::Size`).

use md5::{Digest, Md5};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// A folder of the cache: the longest side of its pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Normal,
    Large,
}

impl Class {
    pub fn px(self) -> u32 {
        match self {
            Class::Normal => 128,
            Class::Large => 256,
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Class::Normal => "normal",
            Class::Large => "large",
        }
    }

    /// The smallest class whose pictures are at least `px` big (the larger for anything over 128).
    pub fn for_px(px: u32) -> Class {
        match px <= 128 {
            true => Class::Normal,
            false => Class::Large,
        }
    }
}

/// What a URI keeps unescaped, as GLib's `g_filename_to_uri` (the others' hash must be ours):
/// unreserved characters, the sub-delimiters, `:` `@` and the slashes.
const URI_KEEP: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');

/// `file:///home/u/a%20b.png` for an absolute `path`.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    format!(
        "file://{}",
        utf8_percent_encode_bytes(path.as_os_str().as_bytes())
    )
}

fn utf8_percent_encode_bytes(b: &[u8]) -> String {
    percent_encoding::percent_encode(b, URI_KEEP).to_string()
}

/// `$XDG_CACHE_HOME/thumbnails`, else `~/.cache/thumbnails`.
pub fn base_dir() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(cache.join("thumbnails"))
}

/// Where the picture of the file at `uri` lives in `base`.
pub fn cache_file(base: &Path, class: Class, uri: &str) -> PathBuf {
    let hash = Md5::digest(uri.as_bytes());
    let name: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    base.join(class.dir()).join(format!("{name}.png"))
}

/// The cached picture's bytes if it was made from this very file: its URI, mtime (whole
/// seconds) and, when it says so, size match.
pub fn read(cached: &Path, uri: &str, mtime: u64, size: u64) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cached).ok()?;
    let reader = png::Decoder::new(std::io::Cursor::new(bytes.as_slice()))
        .read_info()
        .ok()?;
    let text = |k: &str| {
        (reader.info().uncompressed_latin1_text.iter())
            .find(|t| t.keyword == k)
            .map(|t| t.text.clone())
    };
    let same = text("Thumb::URI").is_none_or(|u| u == uri)
        && text("Thumb::MTime")?.parse::<u64>().ok()? == mtime
        && text("Thumb::Size").is_none_or(|s| s.parse::<u64>().ok() == Some(size));
    same.then_some(bytes)
}

/// Writes `rgba` (`w` × `h`) as the picture of the file at `uri` (not replacing a file being
/// read: a part file and a rename); the folders private to the user, as the spec says.
pub fn write(
    cached: &Path,
    (w, h, rgba): (u32, u32, &[u8]),
    uri: &str,
    mtime: u64,
    size: u64,
) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = cached.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // Several pictures are written at once: each its own part file.
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = dir.join(format!(".{}.{n}.part", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&part)?;
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let bad = |e: png::EncodingError| std::io::Error::other(e);
    enc.add_text_chunk("Thumb::URI".into(), uri.into())
        .map_err(bad)?;
    enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string())
        .map_err(bad)?;
    enc.add_text_chunk("Thumb::Size".into(), size.to_string())
        .map_err(bad)?;
    let result = (|| {
        let mut writer = enc.write_header().map_err(bad)?;
        writer.write_image_data(rgba).map_err(bad)?;
        writer.finish().map_err(bad)
    })();
    match result {
        Ok(()) => std::fs::rename(&part, cached),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_escaped_and_the_name_is_the_md5_of_the_uri() {
        assert_eq!(
            uri(Path::new("/home/u/a b+é#.png")),
            "file:///home/u/a%20b+%C3%A9%23.png"
        );
        // The spec's own example.
        let f = cache_file(
            Path::new("/c"),
            Class::Normal,
            "file:///home/jens/photo%20collection/me.png",
        );
        assert!(f.starts_with("/c/normal") && f.extension().is_some_and(|e| e == "png"));
        assert_eq!(f.file_stem().unwrap().len(), 32);
        assert_eq!(Class::for_px(96), Class::Normal);
        assert_eq!(Class::for_px(160), Class::Large);
    }

    #[test]
    fn a_picture_comes_back_only_for_the_file_it_was_made_from() {
        let dir = tempfile::tempdir().unwrap();
        let u = "file:///x/y.png";
        let f = cache_file(dir.path(), Class::Normal, u);
        let px = vec![200u8; 4 * 4 * 4];
        write(&f, (4, 4, &px), u, 1000, 55).unwrap();
        assert!(read(&f, u, 1000, 55).is_some());
        assert!(read(&f, u, 1001, 55).is_none()); // changed
        assert!(read(&f, u, 1000, 56).is_none()); // another size
        assert!(read(&f, "file:///other.png", 1000, 55).is_none());
        assert!(read(&dir.path().join("none.png"), u, 1000, 55).is_none());
        // Private to the user, no part file left.
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(std::fs::read_dir(f.parent().unwrap()).unwrap().count(), 1);
    }
}
