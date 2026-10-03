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

use crate::ops::{self, ErrorChoice, Handler, Method, Report};
use std::cell::Cell;
use std::fs::{self, File, Permissions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Why work on the current archive stopped.
enum Stop {
    Cancel,
    Fail(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        Stop::Fail(e)
    }
}

impl From<zip::result::ZipError> for Stop {
    fn from(e: zip::result::ZipError) -> Self {
        Stop::Fail(e.into())
    }
}

/// Run `op` until it works or the user gives up: `Err(true)` = cancel, `Err(false)` = skip.
fn attempt<T>(
    h: &mut dyn Handler,
    path: &Path,
    mut op: impl FnMut() -> io::Result<T>,
) -> Result<T, bool> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => match h.error(path, &e) {
                ErrorChoice::Retry => {}
                ErrorChoice::Skip => return Err(false),
                ErrorChoice::Cancel => return Err(true),
            },
        }
    }
}

/// Reads a file and remembers the position (the progress bar's "done").
struct Counted {
    f: File,
    pos: Rc<Cell<u64>>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.f.read(buf)?;
        self.pos.set(self.pos.get() + n as u64);
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let n = self.f.seek(to)?;
        self.pos.set(n);
        Ok(n)
    }
}

enum Kind {
    Dir,
    /// Expected size, if the format records it: a short read is a truncated archive.
    File(Option<u64>),
    Symlink(PathBuf),
    Hardlink(PathBuf),
}

/// Writes entries under `root` (inside staging), refusing anything that would land outside it.
struct Stage<'a> {
    root: PathBuf,
    h: &'a mut dyn Handler,
    archive: PathBuf,
    pos: Rc<Cell<u64>>,
    total: u64,
    skipped: bool,
    /// Applied after all entries: writing into a dir changes its mtime, a `r-x` mode blocks writing.
    dirs: Vec<(PathBuf, Option<u32>, Option<SystemTime>)>,
}

impl Stage<'_> {
    fn put(
        &mut self,
        name: &Path,
        kind: Kind,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
        data: &mut dyn Read,
    ) -> Result<(), Stop> {
        if self.h.cancelled() {
            return Err(Stop::Cancel);
        }
        let Some(rel) = safe_path(name) else {
            return self.refuse(name, "unsafe path in archive");
        };
        if !self.parents(&rel)? {
            return self.refuse(name, "path goes through a link in the archive");
        }
        let dst = self.root.join(&rel);
        let old = fs::symlink_metadata(&dst).ok();
        if let Kind::Dir = kind {
            match old {
                Some(m) if m.is_dir() => {}
                Some(_) => {
                    fs::remove_file(&dst)?;
                    fs::create_dir(&dst)?;
                }
                None => fs::create_dir(&dst)?,
            }
            self.dirs.push((dst, mode, mtime));
            return Ok(());
        }
        match old {
            Some(m) if m.is_dir() => {
                return self.refuse(name, "a folder with this name is already in the archive");
            }
            Some(_) => fs::remove_file(&dst)?, // the last entry with a name wins, like tar
            None => {}
        }
        match kind {
            Kind::Symlink(target) => symlink(target, &dst)?,
            Kind::Hardlink(target) => {
                let src = safe_path(&target)
                    .filter(|t| self.parents(t).unwrap_or(false))
                    .map(|t| self.root.join(t));
                match src.and_then(|s| {
                    fs::symlink_metadata(&s)
                        .ok()
                        .filter(|m| m.is_file())
                        .map(|_| s)
                }) {
                    Some(s) => {
                        fs::copy(s, &dst)?;
                    }
                    None => return self.refuse(name, "hard link to a file outside the archive"),
                }
            }
            Kind::File(size) => {
                let mut w = File::options().write(true).create_new(true).open(&dst)?;
                let mut buf = vec![0; 256 << 10];
                let mut written = 0u64;
                loop {
                    if self.h.cancelled() {
                        return Err(Stop::Cancel);
                    }
                    let n = match data.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let _ = fs::remove_file(&dst); // never leave a half-written file
                            return Err(e.into());
                        }
                    };
                    io::Write::write_all(&mut w, &buf[..n])?;
                    written += n as u64;
                    self.h.progress(self.pos.get(), self.total, &self.archive);
                }
                if size.is_some_and(|s| s != written) {
                    let _ = fs::remove_file(&dst);
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
                }
                if let Some(m) = mode {
                    w.set_permissions(Permissions::from_mode(m & 0o777))?;
                }
                if let Some(t) = mtime {
                    w.set_modified(t)?;
                }
            }
            Kind::Dir => unreachable!(),
        }
        Ok(())
    }

    /// Create the missing parents of `rel`; `false` if one of them is not a real dir (a symlink).
    fn parents(&self, rel: &Path) -> io::Result<bool> {
        let mut p = self.root.clone();
        for c in rel.parent().into_iter().flat_map(Path::components) {
            p.push(c);
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&p)?,
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// An entry that must not be written: ask, then skip it (Retry would refuse again).
    fn refuse(&mut self, name: &Path, why: &str) -> Result<(), Stop> {
        self.skipped = true;
        let e = io::Error::new(io::ErrorKind::InvalidData, why);
        match self.h.error(&self.archive.join(name), &e) {
            ErrorChoice::Cancel => Err(Stop::Cancel),
            _ => Ok(()),
        }
    }

    fn finish(&mut self) {
        for (dir, mode, mtime) in self.dirs.drain(..).rev() {
            if let Some(m) = mode {
                let _ = fs::set_permissions(&dir, Permissions::from_mode(m & 0o777 | 0o700));
            }
            if let Some(t) = mtime {
                let _ = File::open(&dir).and_then(|f| f.set_modified(t));
            }
        }
    }
}

fn read_tar(r: impl Read, st: &mut Stage) -> Result<(), Stop> {
    let mut a = tar::Archive::new(r);
    for e in a.entries()? {
        let mut e = e?;
        let name = e.path()?.into_owned();
        let (t, mode, mtime) = {
            let h = e.header();
            (
                h.entry_type(),
                h.mode().ok(),
                h.mtime().ok().map(|s| UNIX_EPOCH + Duration::from_secs(s)),
            )
        };
        let link =
            || -> io::Result<PathBuf> { Ok(e.link_name()?.unwrap_or_default().into_owned()) };
        let kind = if t.is_dir() {
            Kind::Dir
        } else if t.is_file() {
            Kind::File(Some(e.size()))
        } else if t.is_symlink() {
            Kind::Symlink(link()?)
        } else if t.is_hard_link() {
            Kind::Hardlink(link()?)
        } else {
            continue; // devices, fifos, sockets: not created
        };
        st.put(&name, kind, mode, mtime, &mut e)?;
    }
    Ok(())
}

fn read_zip(r: Counted, st: &mut Stage) -> Result<(), Stop> {
    let mut z = zip::ZipArchive::new(r)?;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let name = PathBuf::from(e.name());
        let (mode, mtime) = (e.unix_mode(), e.last_modified().and_then(from_zip_time));
        let kind = if e.is_dir() {
            Kind::Dir
        } else if e.is_symlink() {
            let mut t = String::new();
            e.read_to_string(&mut t)?;
            Kind::Symlink(t.into())
        } else {
            Kind::File(Some(e.size()))
        };
        st.put(&name, kind, mode, mtime, &mut e)?;
    }
    Ok(())
}

/// Zip stores local wall-clock time without a zone: read (and written) in the system zone.
fn from_zip_time(t: zip::DateTime) -> Option<SystemTime> {
    let dt = jiff::civil::DateTime::new(
        t.year() as i16,
        t.month() as i8,
        t.day() as i8,
        t.hour() as i8,
        t.minute() as i8,
        t.second() as i8,
        0,
    )
    .ok()?;
    dt.to_zoned(jiff::tz::TimeZone::system())
        .ok()
        .map(|z| z.timestamp().into())
}

/// p7zip keeps the unix mode in the high 16 bits of the attributes, flagged by 0x8000.
const UNIX_EXTENSION: u32 = 0x8000;
const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

fn read_7z(r: Counted, st: &mut Stage) -> Result<(), Stop> {
    let mut a = sevenz_rust2::ArchiveReader::new(r, sevenz_rust2::Password::empty())
        .map_err(io::Error::other)?;
    let mut stop = None;
    let done = a.for_each_entries(|e, data| {
        if e.is_anti_item() {
            return Ok(true);
        }
        let attrs = e.windows_attributes();
        let mode = (e.has_windows_attributes && attrs & UNIX_EXTENSION != 0).then_some(attrs >> 16);
        let mtime = e
            .has_last_modified_date
            .then(|| e.last_modified_date().into());
        let kind = if e.is_directory() {
            Kind::Dir
        } else if mode.is_some_and(|m| m & S_IFMT == S_IFLNK) {
            let mut t = String::new();
            data.read_to_string(&mut t)?;
            Kind::Symlink(t.into())
        } else {
            Kind::File(Some(e.size()))
        };
        match st.put(Path::new(e.name()), kind, mode, mtime, data) {
            Ok(()) => Ok(true),
            Err(s) => {
                stop = Some(s);
                // Ok(false) does not stop the walk; an error does.
                Err(sevenz_rust2::Error::Other("stopped".into()))
            }
        }
    });
    match (stop, done) {
        (Some(s), _) => Err(s),
        (None, Err(e)) => Err(Stop::Fail(io::Error::other(e))),
        (None, Ok(())) => Ok(()),
    }
}

/// One archive into `st.root`. A single-stream file becomes `stem(archive)` there.
fn read_archive(f: Format, archive: &Path, r: Counted, st: &mut Stage) -> Result<(), Stop> {
    let single = |mut r: Box<dyn Read>, st: &mut Stage| -> Result<(), Stop> {
        let name = archive
            .file_name()
            .map(|n| stem(&n.to_string_lossy()).to_owned())
            .unwrap_or_default();
        let meta = fs::metadata(archive)?;
        st.put(
            Path::new(&name),
            Kind::File(None),
            Some(meta.permissions().mode()),
            meta.modified().ok(),
            &mut r,
        )
    };
    match f {
        Format::Zip => read_zip(r, st),
        Format::SevenZ => read_7z(r, st),
        Format::Tar => read_tar(r, st),
        Format::TarGz => read_tar(flate2::read::MultiGzDecoder::new(r), st),
        Format::TarBz2 => read_tar(bzip2::read::MultiBzDecoder::new(r), st),
        Format::TarXz => read_tar(liblzma::read::XzDecoder::new_multi_decoder(r), st),
        Format::TarZst => read_tar(zstd::stream::read::Decoder::new(r)?, st),
        Format::Gz => single(Box::new(flate2::read::MultiGzDecoder::new(r)), st),
        Format::Bz2 => single(Box::new(bzip2::read::MultiBzDecoder::new(r)), st),
        Format::Xz => single(Box::new(liblzma::read::XzDecoder::new_multi_decoder(r)), st),
        Format::Zst => single(Box::new(zstd::stream::read::Decoder::new(r)?), st),
    }
}

/// Hidden `.shagoff-unpack.<pid>.<n>` in `dest`, created fresh so it is never a user dir.
fn make_staging(dest: &Path) -> io::Result<PathBuf> {
    let mut n = 0u32;
    loop {
        let p = dest.join(format!(".shagoff-unpack.{}.{n}", std::process::id()));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Alt+F9: every archive into one staging dir in `dest` (`staging/<stem>` with `own_dir`), then one
/// `transfer(Move)` puts the top-level entries in place. `completed` = archives unpacked without skips.
pub fn unpack(archives: &[PathBuf], dest: &Path, own_dir: bool, h: &mut dyn Handler) -> Report {
    let mut report = Report::default();
    let staging = match attempt(h, dest, || {
        fs::create_dir_all(dest)?;
        make_staging(dest)
    }) {
        Ok(s) => s,
        Err(cancel) => {
            report.cancelled = cancel;
            return report;
        }
    };
    'archives: for a in archives {
        let name = a
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(f) = Format::detect(&name) else {
            continue;
        };
        let root = if own_dir {
            staging.join(stem(&name))
        } else {
            staging.clone()
        };
        loop {
            let pos = Rc::new(Cell::new(0));
            let (result, skipped) = {
                let mut st = Stage {
                    root: root.clone(),
                    h: &mut *h,
                    archive: a.clone(),
                    pos: pos.clone(),
                    total: 0,
                    skipped: false,
                    dirs: Vec::new(),
                };
                let result = File::open(a).map_err(Stop::from).and_then(|file| {
                    st.total = file.metadata()?.len();
                    fs::create_dir_all(&root)?;
                    read_archive(f, a, Counted { f: file, pos }, &mut st)
                });
                st.finish();
                (result, st.skipped)
            };
            match result {
                Ok(()) => {
                    if !skipped {
                        report.completed.push(a.clone());
                    }
                    break;
                }
                Err(Stop::Cancel) => {
                    report.cancelled = true;
                    break 'archives;
                }
                Err(Stop::Fail(e)) => match h.error(a, &e) {
                    ErrorChoice::Retry => {} // read it again from the start; entries are overwritten
                    ErrorChoice::Skip => break,
                    ErrorChoice::Cancel => {
                        report.cancelled = true;
                        break 'archives;
                    }
                },
            }
        }
    }
    if !report.cancelled {
        let pairs: Vec<(PathBuf, PathBuf)> = fs::read_dir(&staging)
            .map(|rd| {
                rd.flatten()
                    .map(|e| (e.path(), dest.join(e.file_name())))
                    .collect()
            })
            .unwrap_or_default();
        report.cancelled = ops::transfer(Method::Move, &pairs, h).cancelled;
    }
    let _ = fs::remove_dir_all(&staging); // never follows symlinks
    report
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

    use crate::ops::{ErrorChoice, FileInfo, Handler, Resolution};
    use std::fs;
    use std::io;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[derive(Default)]
    struct Script {
        conflicts: Vec<Resolution>,
        errors: Vec<ErrorChoice>,
        errored: Vec<PathBuf>,
        cancel: bool,
    }
    impl Handler for Script {
        fn progress(&mut self, _: u64, _: u64, _: &Path) {}
        fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
            if self.conflicts.is_empty() {
                Resolution::Cancel
            } else {
                self.conflicts.remove(0)
            }
        }
        fn error(&mut self, p: &Path, _: &io::Error) -> ErrorChoice {
            self.errored.push(p.to_path_buf());
            if self.errors.is_empty() {
                ErrorChoice::Cancel
            } else {
                self.errors.remove(0)
            }
        }
        fn cancelled(&self) -> bool {
            self.cancel
        }
    }

    /// A tar with raw headers, so tests can build entries no sane packer writes.
    fn evil_tar(path: &Path, entries: &[(&str, tar::EntryType, &str, &[u8])]) {
        let mut b = tar::Builder::new(fs::File::create(path).unwrap());
        for (name, kind, link, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(*kind);
            h.set_mode(0o644);
            h.set_size(data.len() as u64);
            // set_path refuses "..": write the raw name bytes.
            let raw = &mut h.as_old_mut().name;
            raw[..name.len()].copy_from_slice(name.as_bytes());
            if !link.is_empty() {
                h.set_link_name(link).unwrap();
            }
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        b.finish().unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn unpack_tar_into_dest() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.tar");
        evil_tar(&a, &[("dir/f", tar::EntryType::Regular, "", b"hi")]);
        let out = d.path().join("out");
        let r = unpack(
            std::slice::from_ref(&a),
            &out,
            false,
            &mut Script::default(),
        );
        assert_eq!(r.completed, [a]);
        assert_eq!(fs::read_to_string(out.join("dir/f")).unwrap(), "hi");
        assert_eq!(names(&out), ["dir"]); // staging is gone
    }

    #[test]
    fn unpack_own_dir_uses_the_stem() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("pics.tar");
        evil_tar(&a, &[("f", tar::EntryType::Regular, "", b"x")]);
        unpack(&[a], d.path(), true, &mut Script::default());
        assert_eq!(fs::read_to_string(d.path().join("pics/f")).unwrap(), "x");
    }

    #[test]
    fn evil_dotdot_and_absolute_stay_inside() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("out");
        let a = d.path().join("e.tar");
        evil_tar(
            &a,
            &[
                ("../escaped", tar::EntryType::Regular, "", b"x"),
                ("/abs", tar::EntryType::Regular, "", b"y"),
            ],
        );
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = unpack(&[a], &out, false, &mut h);
        assert!(!d.path().join("escaped").exists());
        assert_eq!(fs::read_to_string(out.join("abs")).unwrap(), "y"); // leading / stripped
        assert_eq!(h.errored.len(), 1);
        assert!(r.completed.is_empty()); // a skip means not completed
    }

    #[test]
    fn evil_symlink_parent_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let a = d.path().join("e.tar");
        evil_tar(
            &a,
            &[
                (
                    "ln",
                    tar::EntryType::Symlink,
                    outside.to_str().unwrap(),
                    b"",
                ),
                ("ln/pwned", tar::EntryType::Regular, "", b"x"),
            ],
        );
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        unpack(&[a], &d.path().join("out"), false, &mut h);
        assert!(!outside.join("pwned").exists());
        assert!(
            fs::symlink_metadata(d.path().join("out/ln"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn evil_hardlink_outside_is_refused() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("secret"), "s").unwrap();
        let a = d.path().join("e.tar");
        evil_tar(&a, &[("h", tar::EntryType::Link, "../secret", b"")]);
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        unpack(&[a], &d.path().join("out"), false, &mut h);
        assert!(!d.path().join("out/h").exists());
    }

    #[test]
    fn hardlink_inside_becomes_a_copy() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("l.tar");
        evil_tar(
            &a,
            &[
                ("f", tar::EntryType::Regular, "", b"data"),
                ("g", tar::EntryType::Link, "f", b""),
            ],
        );
        unpack(&[a], &d.path().join("out"), false, &mut Script::default());
        assert_eq!(fs::read_to_string(d.path().join("out/g")).unwrap(), "data");
    }

    #[test]
    fn devices_and_fifos_are_skipped_silently() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("dev.tar");
        evil_tar(
            &a,
            &[
                ("fifo", tar::EntryType::Fifo, "", b""),
                ("blk", tar::EntryType::Block, "", b""),
                ("ok", tar::EntryType::Regular, "", b"1"),
            ],
        );
        let mut h = Script::default();
        unpack(&[a], &d.path().join("out"), false, &mut h);
        assert_eq!(names(&d.path().join("out")), ["ok"]);
        assert!(h.errored.is_empty());
    }

    #[test]
    fn readonly_dir_in_archive_still_unpacks() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("ro.tar");
        let mut b = tar::Builder::new(fs::File::create(&a).unwrap());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o555);
        h.set_size(0);
        b.append_data(&mut h, "ro", io::empty()).unwrap();
        let mut h = tar::Header::new_gnu();
        h.set_mode(0o644);
        h.set_size(1);
        b.append_data(&mut h, "ro/f", &b"x"[..]).unwrap();
        b.finish().unwrap();
        drop(b);
        let out = d.path().join("out");
        unpack(&[a], &out, false, &mut Script::default());
        assert_eq!(fs::read_to_string(out.join("ro/f")).unwrap(), "x");
        let mode = fs::metadata(out.join("ro")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755); // owner always gets rwx
    }

    #[test]
    fn conflict_skip_keeps_existing() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("c.tar");
        evil_tar(&a, &[("f", tar::EntryType::Regular, "", b"new")]);
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("f"), "old").unwrap();
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Default::default()
        };
        unpack(std::slice::from_ref(&a), &out, false, &mut h);
        assert_eq!(fs::read_to_string(out.join("f")).unwrap(), "old");
        let mut h = Script {
            conflicts: vec![Resolution::Replace],
            ..Default::default()
        };
        unpack(&[a], &out, false, &mut h);
        assert_eq!(fs::read_to_string(out.join("f")).unwrap(), "new");
        assert_eq!(names(&out), ["f"]);
    }

    #[test]
    fn cancel_leaves_no_staging() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("c.tar");
        evil_tar(&a, &[("f", tar::EntryType::Regular, "", b"x")]);
        let out = d.path().join("out");
        let r = unpack(
            &[a],
            &out,
            false,
            &mut Script {
                cancel: true,
                ..Default::default()
            },
        );
        assert!(r.cancelled);
        assert!(names(&out).is_empty());
    }

    #[test]
    fn truncated_archive_skip_keeps_what_was_read() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("t.tar");
        evil_tar(
            &a,
            &[
                ("first", tar::EntryType::Regular, "", b"1"),
                ("second", tar::EntryType::Regular, "", &[7u8; 4096]),
            ],
        );
        let len = fs::metadata(&a).unwrap().len();
        fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_len(len - 3000)
            .unwrap();
        let out = d.path().join("out");
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = unpack(std::slice::from_ref(&a), &out, false, &mut h);
        assert_eq!(h.errored, [a]);
        assert!(r.completed.is_empty());
        assert_eq!(names(&out), ["first"]); // no half-written "second", no staging
    }

    #[test]
    fn single_gz_becomes_a_file() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("notes.txt.gz");
        let mut w = flate2::write::GzEncoder::new(
            fs::File::create(&a).unwrap(),
            flate2::Compression::default(),
        );
        w.write_all(b"text").unwrap();
        w.finish().unwrap();
        unpack(&[a], &d.path().join("out"), false, &mut Script::default());
        assert_eq!(
            fs::read_to_string(d.path().join("out/notes.txt")).unwrap(),
            "text"
        );
    }
}
