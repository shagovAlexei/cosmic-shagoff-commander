//! Pack and unpack archives (Alt+F5 / Alt+F9). Synchronous, driven through `ops::Handler` like `ops`.
//! Unpack writes into a hidden staging dir inside the destination and then moves the entries in
//! place with `ops::transfer(Move)`, so conflicts, part files and cancel behave as for F6.

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
    Gz,
    Bz2,
    Xz,
    Zst,
}

/// Lower-case suffixes, longest match first so "a.tar.gz" is TarGz, not Gz.
const SUFFIXES: [(&str, Format); 16] = [
    (".tar.gz", Format::TarGz),
    (".tar.bz2", Format::TarBz2),
    (".tar.xz", Format::TarXz),
    (".tar.zst", Format::TarZst),
    (".tgz", Format::TarGz),
    (".tbz2", Format::TarBz2),
    (".tbz", Format::TarBz2),
    (".txz", Format::TarXz),
    (".tzst", Format::TarZst),
    (".zip", Format::Zip),
    (".tar", Format::Tar),
    (".7z", Format::SevenZ),
    (".gz", Format::Gz),
    (".bz2", Format::Bz2),
    (".xz", Format::Xz),
    (".zst", Format::Zst),
];

impl Format {
    /// Offered for packing (single-stream formats can't hold a tree).
    pub const PACK: [Format; 7] = [
        Format::Zip,
        Format::Tar,
        Format::TarGz,
        Format::TarBz2,
        Format::TarXz,
        Format::TarZst,
        Format::SevenZ,
    ];

    /// By extension, case-insensitive.
    pub fn detect(name: &str) -> Option<Format> {
        split(name).map(|(_, f)| f)
    }

    /// Canonical extension without the dot.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarBz2 => "tar.bz2",
            Format::TarXz => "tar.xz",
            Format::TarZst => "tar.zst",
            Format::SevenZ => "7z",
            Format::Gz => "gz",
            Format::Bz2 => "bz2",
            Format::Xz => "xz",
            Format::Zst => "zst",
        }
    }

    /// A PACK format by its `ext()` (the config value).
    pub fn from_ext(s: &str) -> Option<Format> {
        Format::PACK.into_iter().find(|f| f.ext() == s)
    }
}

/// (stem, format); the stem is never empty, so ".gz" alone is not an archive.
fn split(name: &str) -> Option<(&str, Format)> {
    let lower = name.to_ascii_lowercase(); // same byte length: the slice below stays on a boundary
    SUFFIXES
        .iter()
        .find(|(s, _)| lower.len() > s.len() && lower.ends_with(s))
        .map(|&(s, f)| (&name[..name.len() - s.len()], f))
}

/// `name` without its archive extension ("a.tar.gz" → "a"); other names unchanged.
pub fn stem(name: &str) -> &str {
    split(name).map_or(name, |(s, _)| s)
}

/// Pack dialog default: one target → its name without the last extension; several → the dir's name.
pub fn archive_name(targets: &[PathBuf], cwd: &Path, f: Format) -> String {
    let base = match targets {
        [one] => one.file_stem().map(|s| s.to_string_lossy().into_owned()),
        _ => cwd.file_name().map(|s| s.to_string_lossy().into_owned()),
    };
    format!("{}.{}", base.unwrap_or_else(|| "archive".into()), f.ext())
}

/// Pack dialog, format changed: replace a PACK extension at the end of `path`, else append.
pub fn with_format(path: &str, f: Format) -> String {
    let base = match split(path) {
        Some((s, old)) if Format::PACK.contains(&old) => s,
        _ => path,
    };
    format!("{base}.{}", f.ext())
}

/// `(sources, archive)` per archive to write. `path` ending in `/` is a dir (default name inside);
/// with `separate`, only the dir part of `path` is used.
pub fn groups(
    targets: &[PathBuf],
    cwd: &Path,
    path: &Path,
    f: Format,
    separate: bool,
) -> Vec<(Vec<PathBuf>, PathBuf)> {
    let is_dir = path.as_os_str().as_encoded_bytes().ends_with(b"/");
    let dir = if is_dir {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    if separate {
        return targets
            .iter()
            .map(|t| {
                let one = vec![t.clone()];
                let name = archive_name(&one, cwd, f);
                (one, dir.join(name))
            })
            .collect();
    }
    let dest = if is_dir {
        path.join(archive_name(targets, cwd, f))
    } else {
        path.to_path_buf()
    };
    vec![(targets.to_vec(), dest)]
}

/// An archive entry name made relative and safe: `None` for empty names and any `..`.
pub fn safe_path(entry: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in entry.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_by_extension() {
        use Format::*;
        for (name, f) in [
            ("a.zip", Zip),
            ("A.ZIP", Zip),
            ("a.tar", Tar),
            ("a.tar.gz", TarGz),
            ("a.tgz", TarGz),
            ("a.tar.bz2", TarBz2),
            ("a.tbz2", TarBz2),
            ("a.tar.xz", TarXz),
            ("a.txz", TarXz),
            ("a.tar.zst", TarZst),
            ("a.tzst", TarZst),
            ("a.7z", SevenZ),
            ("a.gz", Gz),
            ("a.bz2", Bz2),
            ("a.xz", Xz),
            ("a.zst", Zst),
            ("архив.Tar.Gz", TarGz),
        ] {
            assert_eq!(Format::detect(name), Some(f), "{name}");
        }
        for name in ["a.txt", "zip", ".gz", "a.gzip", ""] {
            assert_eq!(Format::detect(name), None, "{name}");
        }
    }

    #[test]
    fn stem_strips_the_format_extension() {
        assert_eq!(stem("a.tar.gz"), "a");
        assert_eq!(stem("a.b.ZIP"), "a.b");
        assert_eq!(stem("notes.txt"), "notes.txt");
    }

    #[test]
    fn ext_round_trips() {
        for f in Format::PACK {
            assert_eq!(Format::from_ext(f.ext()), Some(f));
            assert_eq!(Format::detect(&format!("x.{}", f.ext())), Some(f));
        }
        assert_eq!(Format::from_ext("gz"), None); // not offered for packing
    }

    #[test]
    fn default_archive_name() {
        let cwd = Path::new("/home/u/photos");
        let one = [cwd.join("readme.txt")];
        assert_eq!(archive_name(&one, cwd, Format::Zip), "readme.zip");
        assert_eq!(
            archive_name(&[cwd.join(".bashrc")], cwd, Format::Tar),
            ".bashrc.tar"
        );
        let two = [cwd.join("a"), cwd.join("b")];
        assert_eq!(archive_name(&two, cwd, Format::TarGz), "photos.tar.gz");
        assert_eq!(
            archive_name(&two, Path::new("/"), Format::SevenZ),
            "archive.7z"
        );
    }

    #[test]
    fn with_format_swaps_or_appends() {
        assert_eq!(with_format("/t/a.zip", Format::TarXz), "/t/a.tar.xz");
        assert_eq!(with_format("/t/a.tar.gz", Format::Zip), "/t/a.zip");
        assert_eq!(with_format("/t/a", Format::Zip), "/t/a.zip");
        assert_eq!(with_format("/t/a.gz", Format::Zip), "/t/a.gz.zip"); // .gz is not a PACK format
    }

    #[test]
    fn groups_one_or_separate() {
        let cwd = Path::new("/src");
        let t = vec![cwd.join("a.txt"), cwd.join("dir")];
        assert_eq!(
            groups(&t, cwd, Path::new("/dst/x.zip"), Format::Zip, false),
            [(t.clone(), PathBuf::from("/dst/x.zip"))]
        );
        assert_eq!(
            groups(&t, cwd, Path::new("/dst/x.zip"), Format::Zip, true),
            [
                (vec![t[0].clone()], PathBuf::from("/dst/a.zip")),
                (vec![t[1].clone()], PathBuf::from("/dst/dir.zip")),
            ]
        );
        // Trailing slash: a dir, default name inside it.
        assert_eq!(
            groups(&t, cwd, Path::new("/dst/"), Format::Tar, false),
            [(t.clone(), PathBuf::from("/dst/src.tar"))]
        );
    }

    #[test]
    fn safe_path_rules() {
        assert_eq!(safe_path(Path::new("a/b")), Some("a/b".into()));
        assert_eq!(safe_path(Path::new("./a")), Some("a".into()));
        assert_eq!(safe_path(Path::new("/etc/x")), Some("etc/x".into()));
        assert_eq!(safe_path(Path::new("a/./b/")), Some("a/b".into()));
        assert_eq!(safe_path(Path::new("../x")), None);
        assert_eq!(safe_path(Path::new("a/../../x")), None);
        assert_eq!(safe_path(Path::new("a/../b")), None); // any `..` is refused
        assert_eq!(safe_path(Path::new("")), None);
        assert_eq!(safe_path(Path::new("/")), None);
    }
}
