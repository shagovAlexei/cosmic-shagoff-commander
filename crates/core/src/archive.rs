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

use crate::listing::{Entry, Kind as EntryKind};
use crate::ops::{self, ErrorChoice, Handler, Method, Report};
use std::cell::Cell;
use std::fs::{self, File, Permissions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
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
    /// Extract: only entries under `inner/<name>`, written relative to `inner`.
    select: Option<&'a Select>,
}

/// F5 from an archive panel: the chosen entries of the dir `inner`.
struct Select {
    inner: PathBuf,
    names: Vec<PathBuf>,
}

impl Select {
    fn pick(&self, rel: &Path) -> Option<PathBuf> {
        let r = rel.strip_prefix(&self.inner).ok()?;
        self.names
            .iter()
            .any(|n| r.starts_with(n))
            .then(|| r.to_path_buf())
    }
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
            // "./" or "/": the archive's own root (`tar -C dir -cf x.tar .`), nothing to create.
            let root_only = name
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::CurDir));
            if root_only && !name.as_os_str().is_empty() {
                return Ok(());
            }
            return self.refuse(name, "unsafe path in archive");
        };
        let rel = match self.select {
            None => rel,
            Some(sel) => match sel.pick(&rel) {
                Some(r) => r,
                None => return Ok(()), // not chosen
            },
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
                    .and_then(|t| match self.select {
                        None => Some(t),
                        Some(sel) => sel.pick(&t),
                    })
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
                if let Err(e) = self.write_file(&dst, size, mode, mtime, data) {
                    let _ = fs::remove_file(&dst); // never leave a half-written file
                    return Err(e);
                }
            }
            Kind::Dir => unreachable!(),
        }
        Ok(())
    }

    fn write_file(
        &mut self,
        dst: &Path,
        size: Option<u64>,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
        data: &mut dyn Read,
    ) -> Result<(), Stop> {
        let mut w = File::options().write(true).create_new(true).open(dst)?;
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
                Err(e) => return Err(e.into()),
            };
            io::Write::write_all(&mut w, &buf[..n])?;
            written += n as u64;
            self.h.progress(self.pos.get(), self.total, &self.archive);
        }
        if size.is_some_and(|s| s != written) {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if let Some(m) = mode {
            w.set_permissions(Permissions::from_mode(m & 0o777))?;
        }
        if let Some(t) = mtime {
            w.set_modified(t)?;
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
        } else if t.is_file() || t.is_gnu_sparse() || t.is_contiguous() {
            Kind::File(Some(e.size())) // tar fills a sparse file's holes when reading
        } else if t.is_symlink() {
            Kind::Symlink(link()?)
        } else if t.is_hard_link() {
            Kind::Hardlink(link()?)
        } else {
            // Devices, fifos, sockets: not created, without asking; but not "done" either, so
            // a rewrite of the archive (repack) refuses instead of dropping them.
            st.skipped = true;
            continue;
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

/// Every entry of a tree archive as (inner path, is dir, size, mtime), for Alt+F7. Read afresh, not
/// through the one-archive cache: a search would evict the archive the panel is showing.
// ponytail: dirs that exist only as parents of entries are not listed.
pub fn members(archive: &Path) -> io::Result<Vec<(PathBuf, bool, u64, SystemTime)>> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let f = Format::detect(&name)
        .filter(|f| f.is_tree())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok(read_index(f, archive)?
        .into_iter()
        .map(|i| (i.path, i.dir, i.size, i.mtime))
        .collect())
}

/// Formats that open as folders (single-stream .gz etc. are plain files).
impl Format {
    pub fn is_tree(self) -> bool {
        !matches!(self, Format::Gz | Format::Bz2 | Format::Xz | Format::Zst)
    }
}

/// One entry of an archive's table of contents (no data).
#[derive(Clone, Debug)]
struct Item {
    path: PathBuf,
    dir: bool,
    link: bool,
    size: u64,
    mtime: SystemTime,
    mode: Option<u32>,
}

fn read_index(f: Format, archive: &Path) -> io::Result<Vec<Item>> {
    let mut out = Vec::new();
    let mut push = |name: &Path,
                    dir: bool,
                    link: bool,
                    size: u64,
                    mtime: Option<SystemTime>,
                    mode: Option<u32>| {
        if let Some(path) = safe_path(name) {
            let mtime = mtime.unwrap_or(UNIX_EPOCH);
            out.push(Item {
                path,
                dir,
                link,
                size,
                mtime,
                mode,
            });
        }
    };
    let file = File::open(archive)?;
    match f {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(file)?;
            for i in 0..z.len() {
                let e = z.by_index_raw(i)?;
                let mtime = e.last_modified().and_then(from_zip_time);
                push(
                    Path::new(e.name()),
                    e.is_dir(),
                    e.is_symlink(),
                    e.size(),
                    mtime,
                    e.unix_mode(),
                );
            }
        }
        Format::SevenZ => {
            let a = sevenz_rust2::Archive::read(
                &mut io::BufReader::new(file),
                &sevenz_rust2::Password::empty(),
            )
            .map_err(io::Error::other)?;
            for e in &a.files {
                if e.is_anti_item() {
                    continue;
                }
                let attrs = e.windows_attributes();
                let mode = (e.has_windows_attributes && attrs & UNIX_EXTENSION != 0)
                    .then_some(attrs >> 16);
                let link = mode.is_some_and(|m| m & S_IFMT == S_IFLNK);
                let mtime = e
                    .has_last_modified_date
                    .then(|| e.last_modified_date().into());
                push(
                    Path::new(e.name()),
                    e.is_directory(),
                    link,
                    e.size(),
                    mtime,
                    mode,
                );
            }
        }
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst => {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }
        _ => {
            let r: Box<dyn Read> = match f {
                Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(file)),
                Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
                Format::TarXz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(file)),
                Format::TarZst => Box::new(zstd::stream::read::Decoder::new(file)?),
                _ => Box::new(file),
            };
            let mut a = tar::Archive::new(r);
            for e in a.entries()? {
                let e = e?;
                let h = e.header();
                let t = h.entry_type();
                let file = t.is_file() || t.is_gnu_sparse() || t.is_contiguous();
                if !(t.is_dir() || file || t.is_symlink() || t.is_hard_link()) {
                    continue;
                }
                let mtime = h.mtime().ok().map(|s| UNIX_EPOCH + Duration::from_secs(s));
                push(
                    &e.path()?,
                    t.is_dir(),
                    t.is_symlink(),
                    e.size(),
                    mtime,
                    h.mode().ok(),
                );
            }
        }
    }
    Ok(out)
}

type Index = Arc<Vec<Item>>;

/// The last archive listed: browsing a big tar.gz must not decompress it on every step.
static INDEX: Mutex<Option<(PathBuf, u64, SystemTime, Index)>> = Mutex::new(None);

fn index(archive: &Path) -> io::Result<Index> {
    let m = fs::metadata(archive)?;
    let (size, mtime) = (m.len(), m.modified().unwrap_or(UNIX_EPOCH));
    let mut cache = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, s, t, i)) = cache.as_ref()
        && p == archive
        && *s == size
        && *t == mtime
    {
        return Ok(i.clone());
    }
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let f = Format::detect(&name).ok_or_else(|| io::Error::from(io::ErrorKind::NotADirectory))?;
    let i: Index = Arc::new(read_index(f, archive)?);
    *cache = Some((archive.to_path_buf(), size, mtime, i.clone()));
    Ok(i)
}

/// "/x/a.zip/docs" → ("/x/a.zip", "docs"). `None` if `p` is a real dir or no ancestor (up to the
/// first real dir) is a regular file with a tree format.
pub fn split_path(p: &Path) -> Option<(PathBuf, PathBuf)> {
    // Rebuilt from components: "a.zip/" with its trailing slash fails `metadata` (ENOTDIR).
    let p: PathBuf = p.components().collect();
    let p = p.as_path();
    for a in p.ancestors() {
        let Ok(m) = fs::metadata(a) else { continue };
        if m.is_dir() {
            return None;
        }
        let name = a.file_name()?.to_string_lossy();
        return Format::detect(&name)
            .filter(|f| m.is_file() && f.is_tree())
            .and_then(|_| Some((a.to_path_buf(), p.strip_prefix(a).ok()?.to_path_buf())));
    }
    None
}

/// Direct children of `inner` inside `archive`, as listing entries (no ".."). Dirs that have no
/// entry of their own (`a/b/f` without `a/`) are synthesized.
pub fn list(archive: &Path, inner: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let items = index(archive)?;
    let mut found = inner.as_os_str().is_empty();
    let mut kids: std::collections::BTreeMap<std::ffi::OsString, Entry> = Default::default();
    for it in items.iter() {
        if it.path == inner {
            if !it.dir {
                return Err(io::ErrorKind::NotADirectory.into());
            }
            found = true;
            continue;
        }
        let Ok(rest) = it.path.strip_prefix(inner) else {
            continue;
        };
        let mut parts = rest.components();
        let Some(first) = parts.next() else { continue };
        found = true;
        let os_name = first.as_os_str().to_os_string();
        let name = os_name.to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let own = parts.next().is_none();
        let dir = !own || it.dir;
        let entry = Entry {
            ext: if dir {
                String::new()
            } else {
                crate::listing::ext_of(&name)
            },
            os_name: os_name.clone(),
            name,
            size: if dir { 0 } else { it.size },
            mtime: it.mtime,
            kind: if dir { EntryKind::Dir } else { EntryKind::File },
            is_link: own && it.link,
            mode: it
                .mode
                .map_or(if dir { 0o755 } else { 0o644 }, |m| m & 0o7777),
            owner: None,
            target: None,
        };
        match kids.get_mut(&os_name) {
            // An implicit dir takes the newest mtime inside; an own entry replaces it.
            Some(old) if !own => old.mtime = old.mtime.max(it.mtime),
            Some(old) => *old = entry,
            None => {
                kids.insert(os_name, entry);
            }
        }
    }
    if !found {
        return Err(io::ErrorKind::NotFound.into());
    }
    Ok(kids.into_values().collect())
}

/// Where files opened from archives (Enter, F3) are extracted: the user's cache dir, on disk —
/// `$XDG_RUNTIME_DIR` is a small RAM tmpfs that other session services need.
pub fn temp_root() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(std::env::temp_dir);
    temp_root_in(std::env::var_os("XDG_CACHE_HOME"), &home)
}

fn temp_root_in(cache: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    match cache {
        Some(c) if !c.is_empty() => PathBuf::from(c),
        _ => home.join(".cache"),
    }
    .join("shagoff-commander")
}

/// A new empty `<root>/<pid>/<n>`: same-named files opened one after another never mix.
pub fn fresh_temp_dir(root: &Path) -> io::Result<PathBuf> {
    let mine = root.join(std::process::id().to_string());
    fs::create_dir_all(&mine)?;
    let mut n = 0u32;
    loop {
        let p = mine.join(n.to_string());
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// At startup: remove `<root>/<pid>` left by processes that are gone (crashed, killed).
pub fn clean_temp(root: &Path) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if !Path::new("/proc").join(pid.to_string()).exists() {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

/// Hidden `.shagoff-unpack.<pid>.<n>` in `dest`, created fresh so it is never a user dir.
pub(crate) fn make_staging(dest: &Path) -> io::Result<PathBuf> {
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

/// One archive to read in a `run`.
struct Unit {
    archive: PathBuf,
    /// Subdir of staging for "each archive into its own folder".
    sub: Option<PathBuf>,
    select: Option<Select>,
    /// Reported as `completed` when the archive was read without skips.
    done: Vec<PathBuf>,
}

/// Alt+F9: every archive into one staging dir in `dest` (`staging/<stem>` with `own_dir`), then one
/// `transfer(Move)` puts the top-level entries in place. `completed` = archives unpacked without skips.
pub fn unpack(archives: &[PathBuf], dest: &Path, own_dir: bool, h: &mut dyn Handler) -> Report {
    let units: Vec<Unit> = archives
        .iter()
        .map(|a| {
            let name = a
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // A stem like ".." ("...tar") must not climb out of staging: use the whole name then.
            let sub = own_dir.then(|| match safe_path(Path::new(stem(&name))) {
                Some(s) if s.components().count() == 1 => s,
                _ => PathBuf::from(&name),
            });
            Unit {
                archive: a.clone(),
                sub,
                select: None,
                done: vec![a.clone()],
            }
        })
        .collect();
    run(&units, dest, h)
}

/// F5 / Ctrl+C / Enter in an archive panel: `names` (relative to the dir `inner`) into `dest`.
/// `completed` = their paths through the archive (`archive/inner/name`) when nothing was skipped.
pub fn extract(
    archive: &Path,
    inner: &Path,
    names: &[PathBuf],
    dest: &Path,
    h: &mut dyn Handler,
) -> Report {
    let unit = Unit {
        archive: archive.to_path_buf(),
        sub: None,
        select: Some(Select {
            inner: inner.to_path_buf(),
            names: names.to_vec(),
        }),
        done: names.iter().map(|n| archive.join(inner).join(n)).collect(),
    };
    run(std::slice::from_ref(&unit), dest, h)
}

fn run(units: &[Unit], dest: &Path, h: &mut dyn Handler) -> Report {
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
    // Units read without skips; `completed` once their entries also got past the final move.
    let mut read: Vec<&Unit> = Vec::new();
    'archives: for u in units {
        let a = &u.archive;
        let name = a
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(f) = Format::detect(&name) else {
            continue;
        };
        let root = match &u.sub {
            Some(sub) => staging.join(sub),
            None => staging.clone(),
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
                    select: u.select.as_ref(),
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
                        read.push(u);
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
        let t = ops::transfer(Method::Move, &pairs, h);
        report.cancelled = t.cancelled;
        // A conflict answered Skip leaves the entry unextracted: F6 out must not delete it.
        let moved = |p: &Path| t.completed.iter().any(|c| c == p);
        for u in read {
            match (&u.select, &u.sub) {
                (Some(sel), _) => report.completed.extend(
                    sel.names
                        .iter()
                        .zip(&u.done)
                        .filter(|(n, _)| moved(&staging.join(n)))
                        .map(|(_, d)| d.clone()),
                ),
                (None, Some(sub)) if moved(&staging.join(sub)) => {
                    report.completed.extend(u.done.iter().cloned())
                }
                (None, None) if pairs.iter().all(|(src, _)| moved(src)) => {
                    report.completed.extend(u.done.iter().cloned())
                }
                _ => {}
            }
        }
    }
    let _ = fs::remove_dir_all(&staging); // never follows symlinks
    report
}

use crate::ops::{FileInfo, Resolution};
use std::fs::Metadata;
use std::io::Write;
use std::os::unix::fs::MetadataExt;

/// Compression around a tar stream; `finish` flushes the trailer the encoder needs.
enum Enc {
    Plain(File),
    Gz(flate2::write::GzEncoder<File>),
    Bz2(bzip2::write::BzEncoder<File>),
    Xz(liblzma::write::XzEncoder<File>),
    Zst(zstd::stream::write::Encoder<'static, File>),
}

impl Write for Enc {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Enc::Plain(w) => w.write(b),
            Enc::Gz(w) => w.write(b),
            Enc::Bz2(w) => w.write(b),
            Enc::Xz(w) => w.write(b),
            Enc::Zst(w) => w.write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Enc::Plain(w) => w.flush(),
            Enc::Gz(w) => w.flush(),
            Enc::Bz2(w) => w.flush(),
            Enc::Xz(w) => w.flush(),
            Enc::Zst(w) => w.flush(),
        }
    }
}

impl Enc {
    fn finish(self) -> io::Result<File> {
        match self {
            Enc::Plain(w) => Ok(w),
            Enc::Gz(w) => w.finish(),
            Enc::Bz2(w) => w.finish(),
            Enc::Xz(w) => w.finish(),
            Enc::Zst(w) => w.finish(),
        }
    }
}

enum Packer {
    Zip(Box<zip::ZipWriter<File>>),
    Tar(tar::Builder<Enc>),
    SevenZ(sevenz_rust2::ArchiveWriter<File>),
}

impl Packer {
    fn new(f: Format, w: File) -> io::Result<Packer> {
        let tar = |e| Packer::Tar(tar::Builder::new(e));
        Ok(match f {
            Format::Zip => Packer::Zip(Box::new(zip::ZipWriter::new(w))),
            Format::SevenZ => {
                Packer::SevenZ(sevenz_rust2::ArchiveWriter::new(w).map_err(io::Error::other)?)
            }
            Format::Tar => tar(Enc::Plain(w)),
            Format::TarGz => tar(Enc::Gz(flate2::write::GzEncoder::new(
                w,
                flate2::Compression::default(),
            ))),
            Format::TarBz2 => tar(Enc::Bz2(bzip2::write::BzEncoder::new(
                w,
                bzip2::Compression::default(),
            ))),
            Format::TarXz => tar(Enc::Xz(liblzma::write::XzEncoder::new(w, 6))),
            Format::TarZst => tar(Enc::Zst(zstd::stream::write::Encoder::new(w, 0)?)),
            Format::Gz | Format::Bz2 | Format::Xz | Format::Zst => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "not a packing format",
                ));
            }
        })
    }

    /// Zip and 7z store names as UTF-8 only.
    fn needs_utf8(&self) -> bool {
        !matches!(self, Packer::Tar(_))
    }

    fn dir(&mut self, rel: &Path, m: &Metadata) -> io::Result<()> {
        match self {
            Packer::Zip(z) => Ok(z.add_directory(utf8(rel)?, zip_opts(m))?),
            Packer::Tar(b) => {
                let mut h = tar_header(m);
                h.set_size(0);
                b.append_data(&mut h, rel, io::empty())
            }
            Packer::SevenZ(w) => {
                w.push_archive_entry::<&[u8]>(sz_entry(utf8(rel)?, m, true), None)
                    .map_err(io::Error::other)?;
                Ok(())
            }
        }
    }

    fn file(&mut self, rel: &Path, m: &Metadata, r: &mut dyn Read) -> io::Result<()> {
        match self {
            Packer::Zip(z) => {
                z.start_file(
                    utf8(rel)?,
                    zip_opts(m).large_file(m.len() >= u32::MAX as u64),
                )?;
                io::copy(r, z)?;
                Ok(())
            }
            Packer::Tar(b) => {
                let exact = Exact { r, left: m.len() };
                b.append_data(&mut tar_header(m), rel, exact)
            }
            Packer::SevenZ(w) => {
                w.push_archive_entry(sz_entry(utf8(rel)?, m, false), Some(r))
                    .map_err(io::Error::other)?;
                Ok(())
            }
        }
    }

    /// `Ok(false)`: this format can't hold the link (7z, or a non-UTF-8 target in zip).
    fn symlink(&mut self, rel: &Path, target: &Path, m: &Metadata) -> io::Result<bool> {
        match self {
            Packer::Zip(z) => match target.to_str() {
                Some(t) => {
                    z.add_symlink(utf8(rel)?, t, zip_opts(m))?;
                    Ok(true)
                }
                None => Ok(false),
            },
            Packer::Tar(b) => {
                let mut h = tar_header(m);
                h.set_size(0);
                b.append_link(&mut h, rel, target)?;
                Ok(true)
            }
            Packer::SevenZ(_) => Ok(false),
        }
    }

    fn finish(self) -> io::Result<File> {
        match self {
            Packer::Zip(z) => Ok(z.finish()?),
            Packer::Tar(b) => b.into_inner()?.finish(),
            Packer::SevenZ(w) => w.finish(),
        }
    }
}

fn utf8(rel: &Path) -> io::Result<&str> {
    rel.to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "name is not UTF-8"))
}

fn tar_header(m: &Metadata) -> tar::Header {
    let mut h = tar::Header::new_gnu();
    h.set_metadata_in_mode(m, tar::HeaderMode::Complete);
    h
}

fn zip_opts(m: &Metadata) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(m.mode() & 0o7777)
        .last_modified_time(m.modified().map(to_zip_time).unwrap_or_default())
}

fn sz_entry(rel: &str, m: &Metadata, dir: bool) -> sevenz_rust2::ArchiveEntry {
    let mut e = if dir {
        sevenz_rust2::ArchiveEntry::new_directory(rel)
    } else {
        sevenz_rust2::ArchiveEntry::new_file(rel)
    };
    e.has_windows_attributes = true;
    e.windows_attributes = UNIX_EXTENSION | (m.mode() << 16) | if dir { 0x10 } else { 0 };
    if let Some(t) = m.modified().ok().and_then(|t| t.try_into().ok()) {
        e.last_modified_date = t;
        e.has_last_modified_date = true;
    }
    e
}

/// Zip stores local wall-clock time without a zone: written and read in the system zone.
fn to_zip_time(t: SystemTime) -> zip::DateTime {
    jiff::Timestamp::try_from(t)
        .ok()
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()))
        .and_then(|z| {
            zip::DateTime::from_date_and_time(
                z.year() as u16,
                z.month() as u8,
                z.day() as u8,
                z.hour() as u8,
                z.minute() as u8,
                z.second() as u8,
            )
            .ok()
        })
        .unwrap_or_default()
}

/// Exactly `left` bytes: tar has written the size into the header already, so a file that
/// shrank since must fail (tar would pad it silently) and one that grew is cut.
struct Exact<R> {
    r: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let max = buf.len().min(self.left.try_into().unwrap_or(usize::MAX));
        let n = self.r.read(&mut buf[..max])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// Counts bytes into the progress bar and stops reading on cancel.
struct Progress<'a> {
    r: File,
    h: &'a mut dyn Handler,
    done: &'a mut u64,
    total: u64,
    path: &'a Path,
    cancelled: bool,
}

impl Read for Progress<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.h.cancelled() {
            self.cancelled = true;
            return Err(io::Error::other("cancelled"));
        }
        let n = self.r.read(buf)?;
        *self.done += n as u64;
        self.h.progress(*self.done, self.total, self.path);
        Ok(n)
    }
}

/// Everything under `p`, dirs before their contents, symlinks not followed. `skip`: (dev, ino) of
/// the part file and the target archive, so an archive inside a packed dir never packs itself.
fn walk(
    p: &Path,
    skip: &[(u64, u64)],
    out: &mut Vec<(PathBuf, Metadata)>,
    h: &mut dyn Handler,
) -> Result<bool, bool> {
    let m = attempt(h, p, || fs::symlink_metadata(p))?;
    if skip.contains(&(m.dev(), m.ino())) {
        return Ok(true);
    }
    let is_dir = m.is_dir();
    let special = !is_dir && !m.is_file() && !m.file_type().is_symlink();
    if special {
        return Ok(false); // sockets, fifos, devices: not packed (and not "completed")
    }
    out.push((p.to_path_buf(), m));
    let mut ok = true;
    if is_dir {
        let mut children: Vec<PathBuf> = match attempt(h, p, || {
            fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect()
        }) {
            Ok(c) => c,
            Err(false) => return Ok(false),
            Err(true) => return Err(true),
        };
        children.sort();
        for c in children {
            ok &= walk(&c, skip, out, h)?;
        }
    }
    Ok(ok)
}

/// What became of one group.
enum Packed {
    Done { skipped: bool },
    Dropped,
    Cancel,
}

/// Alt+F5: one archive per `(sources, archive)`; names inside are relative to `base`.
/// `completed` = sources of groups written without any skip (for "move to archive").
pub fn pack(
    f: Format,
    base: &Path,
    groups: &[(Vec<PathBuf>, PathBuf)],
    h: &mut dyn Handler,
) -> Report {
    let mut report = Report::default();
    let mut policy: Option<Resolution> = None;
    for (sources, dest) in groups {
        match pack_one(f, base, sources, dest, &mut policy, h) {
            // An archive inside a source dir would be deleted with it by "move to archive".
            Packed::Done { skipped: false } if !inside_any(dest, sources) => {
                report.completed.extend(sources.iter().cloned())
            }
            Packed::Done { .. } => {}
            Packed::Dropped => {}
            Packed::Cancel => {
                report.cancelled = true;
                break;
            }
        }
    }
    report
}

/// `dest` lies inside one of the source dirs (symlinks resolved).
fn inside_any(dest: &Path, sources: &[PathBuf]) -> bool {
    let Some(dir) = dest.parent().and_then(|p| fs::canonicalize(p).ok()) else {
        return false;
    };
    sources.iter().any(|s| {
        fs::symlink_metadata(s).is_ok_and(|m| m.is_dir())
            && fs::canonicalize(s).is_ok_and(|s| dir.starts_with(s))
    })
}

/// `dest` already exists and is one of the sources: replacing it would destroy what is packed.
fn is_a_source(dest: &Path, sources: &[PathBuf]) -> bool {
    let id = |p: &Path| fs::symlink_metadata(p).ok().map(|m| (m.dev(), m.ino()));
    id(dest).is_some_and(|d| sources.iter().any(|s| id(s) == Some(d)))
}

fn pack_one(
    f: Format,
    base: &Path,
    sources: &[PathBuf],
    dest: &Path,
    policy: &mut Option<Resolution>,
    h: &mut dyn Handler,
) -> Packed {
    if is_a_source(dest, sources) {
        let e = io::Error::new(
            io::ErrorKind::InvalidInput,
            "the archive would replace a file being packed",
        );
        return match h.error(dest, &e) {
            ErrorChoice::Cancel => Packed::Cancel,
            _ => Packed::Dropped,
        };
    }
    loop {
        let (part, w) = match attempt(h, dest, || {
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir)?;
            }
            ops::create_part(dest)
        }) {
            Ok(pw) => pw,
            Err(true) => return Packed::Cancel,
            Err(false) => return Packed::Dropped,
        };
        let outcome = write_archive(f, base, sources, dest, &part, w, h);
        match outcome {
            Ok(Some(skipped)) => {
                return match place(&part, dest, policy, h) {
                    Some(true) => Packed::Done { skipped },
                    Some(false) => Packed::Dropped,
                    None => Packed::Cancel,
                };
            }
            Ok(None) => {
                let _ = fs::remove_file(&part);
                return Packed::Cancel;
            }
            Err(e) => {
                let _ = fs::remove_file(&part);
                match h.error(dest, &e) {
                    ErrorChoice::Retry => {}
                    ErrorChoice::Skip => return Packed::Dropped,
                    ErrorChoice::Cancel => return Packed::Cancel,
                }
            }
        }
    }
}

/// Write the archive into `part`. `Ok(Some(skipped))` = written; `Ok(None)` = cancelled.
fn write_archive(
    f: Format,
    base: &Path,
    sources: &[PathBuf],
    dest: &Path,
    part: &Path,
    w: File,
    h: &mut dyn Handler,
) -> io::Result<Option<bool>> {
    let mut skip = vec![];
    for p in [part, dest] {
        if let Ok(m) = fs::symlink_metadata(p) {
            skip.push((m.dev(), m.ino()));
        }
    }
    let mut items = Vec::new();
    let mut skipped = false;
    for s in sources {
        match walk(s, &skip, &mut items, h) {
            Ok(ok) => skipped |= !ok,
            Err(_) => return Ok(None), // walk only gives up on Cancel; Skip is Ok(false)
        }
    }
    let total = items
        .iter()
        .filter(|(_, m)| m.is_file())
        .map(|(_, m)| m.len())
        .sum();
    let mut done = 0;
    let mut p = Packer::new(f, w)?;
    for (path, m) in &items {
        if h.cancelled() {
            return Ok(None);
        }
        let rel = path.strip_prefix(base).unwrap_or(path);
        if p.needs_utf8() && rel.to_str().is_none() {
            let e = io::Error::new(io::ErrorKind::InvalidData, "name is not UTF-8");
            match h.error(path, &e) {
                ErrorChoice::Cancel => return Ok(None),
                _ => {
                    skipped = true;
                    continue;
                }
            }
        }
        if m.is_dir() {
            p.dir(rel, m)?;
        } else if m.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            skipped |= !p.symlink(rel, &target, m)?;
        } else {
            let r = match attempt(h, path, || File::open(path)) {
                Ok(r) => r,
                Err(true) => return Ok(None),
                Err(false) => {
                    skipped = true;
                    done += m.len();
                    continue;
                }
            };
            let mut pr = Progress {
                r,
                h: &mut *h,
                done: &mut done,
                total,
                path,
                cancelled: false,
            };
            let written = p.file(rel, m, &mut pr);
            if pr.cancelled {
                return Ok(None);
            }
            written?;
        }
    }
    p.finish()?;
    Ok(Some(skipped))
}

/// Rename `part` to `dest` without replacing; an existing `dest` asks. `Some(placed)`, `None` = cancel.
fn place(
    part: &Path,
    dest: &Path,
    policy: &mut Option<Resolution>,
    h: &mut dyn Handler,
) -> Option<bool> {
    let drop_part = || {
        let _ = fs::remove_file(part);
    };
    let info = |p: &Path| {
        let m = fs::symlink_metadata(p).ok();
        FileInfo {
            path: p.to_path_buf(),
            size: m.as_ref().map_or(0, Metadata::len),
            mtime: m
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        }
    };
    let mut dest = dest.to_path_buf();
    loop {
        match ops::rename_noreplace(part, &dest) {
            Ok(()) => return Some(true),
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
                return match attempt(h, &dest, || ops::rename_noreplace(part, &dest)) {
                    Ok(()) => Some(true),
                    Err(cancel) => {
                        drop_part();
                        (!cancel).then_some(false)
                    }
                };
            }
            Err(_) => {}
        }
        let answer = policy
            .clone()
            .unwrap_or_else(|| h.conflict(&info(part), &info(&dest)));
        if matches!(
            answer,
            Resolution::ReplaceAll
                | Resolution::SkipAll
                | Resolution::ReplaceOlder
                | Resolution::RenameAll
        ) {
            *policy = Some(answer.clone());
        }
        match answer {
            // The new archive is always the newer file.
            Resolution::Replace | Resolution::ReplaceAll | Resolution::ReplaceOlder => {
                return match attempt(h, &dest, || fs::rename(part, &dest)) {
                    Ok(()) => Some(true),
                    Err(cancel) => {
                        drop_part();
                        (!cancel).then_some(false)
                    }
                };
            }
            // Asked again if the new name is taken too.
            Resolution::Rename(name) if ops::valid_name(&name) => dest.set_file_name(name),
            Resolution::RenameAll => dest = ops::unique_name(&dest),
            Resolution::Skip | Resolution::SkipAll | Resolution::Rename(_) => {
                drop_part();
                return Some(false);
            }
            Resolution::Cancel => {
                drop_part();
                return None;
            }
        }
    }
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
        let r = unpack(
            std::slice::from_ref(&a),
            &d.path().join("out"),
            false,
            &mut h,
        );
        assert_eq!(names(&d.path().join("out")), ["ok"]);
        assert!(h.errored.is_empty());
        // Not "done": a rewrite of the archive (repack) would drop them.
        assert!(r.completed.is_empty());
    }

    #[test]
    fn regression_extract_skipped_on_conflict_is_not_completed() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.tar");
        evil_tar(&a, &[("f.txt", tar::EntryType::Regular, "", b"new")]);
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("f.txt"), "old").unwrap();
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Script::default()
        };
        let r = extract(&a, Path::new(""), &[PathBuf::from("f.txt")], &out, &mut h);
        assert_eq!(fs::read_to_string(out.join("f.txt")).unwrap(), "old");
        assert!(r.completed.is_empty(), "{:?}", r.completed);
    }

    #[test]
    fn regression_gnu_sparse_file_is_unpacked() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sparse.img");
        let file = fs::File::create(&f).unwrap();
        file.set_len(1 << 20).unwrap();
        use std::io::{Seek, Write};
        let mut file = file;
        file.seek(io::SeekFrom::Start(4096)).unwrap();
        file.write_all(b"data").unwrap();
        drop(file);
        let a = d.path().join("s.tar");
        // GNU tar is what writes sparse entries; skip where it is missing.
        let Ok(st) = std::process::Command::new("tar")
            .args(["--format=gnu", "-cSf"])
            .arg(&a)
            .arg("-C")
            .arg(d.path())
            .arg("sparse.img")
            .status()
        else {
            return;
        };
        assert!(st.success());
        fs::remove_file(&f).unwrap();
        let out = d.path().join("out");
        let r = unpack(
            std::slice::from_ref(&a),
            &out,
            false,
            &mut Script::default(),
        );
        assert_eq!(r.completed, [a]);
        let back = fs::read(out.join("sparse.img")).unwrap();
        assert_eq!(back.len(), 1 << 20);
        assert_eq!(&back[4096..4100], b"data");
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

    use std::time::{Duration, SystemTime};

    /// file, nested dir, empty dir, symlink, Cyrillic name; fixed mtime and modes.
    fn tree(base: &Path) -> Vec<PathBuf> {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let src = base.join("src");
        fs::create_dir_all(src.join("sub/empty")).unwrap();
        fs::write(src.join("sub/файл.txt"), "привет").unwrap();
        fs::write(src.join("run.sh"), "#!/bin/sh").unwrap();
        fs::set_permissions(src.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
        symlink("run.sh", src.join("ln")).unwrap();
        for p in ["sub/файл.txt", "run.sh"] {
            fs::File::options()
                .write(true)
                .open(src.join(p))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        vec![src]
    }

    fn mtime(p: &Path) -> u64 {
        fs::metadata(p)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn round_trip_every_pack_format() {
        for f in Format::PACK {
            let d = tempfile::tempdir().unwrap();
            let sources = tree(d.path());
            let archive = d.path().join(format!("a.{}", f.ext()));
            let r = pack(
                f,
                d.path(),
                &[(sources.clone(), archive.clone())],
                &mut Script::default(),
            );
            assert!(!r.cancelled, "{f:?}");
            let out = d.path().join("out");
            let r = unpack(
                std::slice::from_ref(&archive),
                &out,
                false,
                &mut Script::default(),
            );
            assert_eq!(r.completed, [archive], "{f:?}");
            let o = out.join("src");
            assert_eq!(
                fs::read_to_string(o.join("sub/файл.txt")).unwrap(),
                "привет",
                "{f:?}"
            );
            assert!(o.join("sub/empty").is_dir(), "{f:?}");
            assert_eq!(
                fs::metadata(o.join("run.sh")).unwrap().permissions().mode() & 0o777,
                0o750,
                "{f:?}"
            );
            assert!(
                mtime(&o.join("run.sh")).abs_diff(1_700_000_000) <= 2,
                "{f:?}"
            ); // zip: 2 s DOS time
            if f != Format::SevenZ {
                assert_eq!(
                    fs::read_link(o.join("ln")).unwrap(),
                    Path::new("run.sh"),
                    "{f:?}"
                );
            }
        }
    }

    #[test]
    fn seven_z_symlink_is_a_skip() {
        let d = tempfile::tempdir().unwrap();
        let sources = tree(d.path());
        let r = pack(
            Format::SevenZ,
            d.path(),
            &[(sources, d.path().join("a.7z"))],
            &mut Script::default(),
        );
        assert!(r.completed.is_empty()); // so "move to archive" keeps the sources
        assert!(d.path().join("a.7z").exists());
    }

    #[test]
    fn completed_excludes_skipped_group() {
        let d = tempfile::tempdir().unwrap();
        let (ok, bad) = (d.path().join("ok"), d.path().join("bad"));
        fs::write(&ok, "1").unwrap();
        fs::write(&bad, "2").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&bad).is_ok() {
            return; // running as root: can't make a file unreadable
        }
        let groups = [
            (vec![ok.clone()], d.path().join("ok.zip")),
            (vec![bad.clone()], d.path().join("bad.zip")),
        ];
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = pack(Format::Zip, d.path(), &groups, &mut h);
        assert_eq!(r.completed, [ok]);
        assert_eq!(h.errored, [bad]);
    }

    #[test]
    fn archive_inside_source_is_not_packed() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("f"), "x").unwrap();
        let archive = src.join("self.tar");
        pack(
            Format::Tar,
            d.path(),
            &[(vec![src.clone()], archive.clone())],
            &mut Script::default(),
        );
        let names: Vec<String> = tar::Archive::new(fs::File::open(&archive).unwrap())
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .all(|n| !n.contains("self.tar") && !n.contains("shagoff-part")),
            "{names:?}"
        );
    }

    #[test]
    fn existing_archive_asks() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("f"), "x").unwrap();
        let archive = d.path().join("a.zip");
        fs::write(&archive, "old").unwrap();
        let g = [(vec![d.path().join("f")], archive.clone())];
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Default::default()
        };
        pack(Format::Zip, d.path(), &g, &mut h);
        assert_eq!(fs::read_to_string(&archive).unwrap(), "old");
        let mut h = Script {
            conflicts: vec![Resolution::Replace],
            ..Default::default()
        };
        pack(Format::Zip, d.path(), &g, &mut h);
        assert_ne!(fs::read(&archive).unwrap(), b"old");
        assert!(names(d.path()).iter().all(|n| !n.contains("shagoff-part")));
    }

    #[test]
    fn cancel_pack_leaves_no_part() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("f"), "x").unwrap();
        let g = [(vec![d.path().join("f")], d.path().join("a.tar.gz"))];
        let r = pack(
            Format::TarGz,
            d.path(),
            &g,
            &mut Script {
                cancel: true,
                ..Default::default()
            },
        );
        assert!(r.cancelled);
        assert_eq!(names(d.path()), ["f"]);
    }

    #[test]
    fn regression_archive_inside_source_is_not_completed() {
        // "Move to archive" deletes `completed`: it must never take the new archive with it.
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("f"), "x").unwrap();
        let g = [(vec![src.clone()], src.join("src.zip"))];
        let r = pack(Format::Zip, d.path(), &g, &mut Script::default());
        assert!(r.completed.is_empty());
        assert!(src.join("src.zip").exists());
    }

    #[test]
    fn regression_dotdot_stem_stays_in_staging() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("f"), "old").unwrap();
        let a = d.path().join("...tar"); // stem ".."
        evil_tar(&a, &[("f", tar::EntryType::Regular, "", b"new")]);
        unpack(&[a], &out, true, &mut Script::default());
        assert_eq!(fs::read_to_string(out.join("f")).unwrap(), "old");
        assert_eq!(fs::read_to_string(out.join("...tar/f")).unwrap(), "new");
    }

    #[test]
    fn regression_dot_slash_root_entry_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.tar");
        evil_tar(
            &a,
            &[
                ("./", tar::EntryType::Directory, "", b""),
                ("./f", tar::EntryType::Regular, "", b"x"),
            ],
        );
        let mut h = Script::default();
        let r = unpack(
            std::slice::from_ref(&a),
            &d.path().join("out"),
            false,
            &mut h,
        );
        assert!(h.errored.is_empty(), "{:?}", h.errored);
        assert_eq!(r.completed, [a]);
    }

    #[test]
    fn regression_pack_onto_one_of_its_sources_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        fs::write(&a, "precious").unwrap();
        let mut h = Script {
            conflicts: vec![Resolution::Replace],
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        let r = pack(
            Format::Zip,
            d.path(),
            &[(vec![a.clone()], a.clone())],
            &mut h,
        );
        assert_eq!(fs::read_to_string(&a).unwrap(), "precious");
        assert!(r.completed.is_empty());
        assert_eq!(h.errored, [a]);
    }

    #[test]
    fn regression_short_source_is_an_error_not_a_padded_entry() {
        // A file that shrank between the walk and the read: tar would pad silently.
        let mut r = Exact {
            r: &b"abc"[..],
            left: 5,
        };
        let e = io::copy(&mut r, &mut io::sink()).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
        let mut r = Exact {
            r: &b"abcdef"[..],
            left: 3,
        }; // grew: only the recorded size
        let mut v = Vec::new();
        io::copy(&mut r, &mut v).unwrap();
        assert_eq!(v, b"abc");
    }

    /// zip with `docs/a.txt`, `docs/img/p.png`, `.hidden`, `top.txt` — no explicit dir entries.
    fn zip_fixture(path: &Path) {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for (n, d) in [
            ("docs/a.txt", "aa"),
            ("docs/img/p.png", "png"),
            (".hidden", "h"),
            ("top.txt", "t"),
        ] {
            z.start_file(n, o).unwrap();
            z.write_all(d.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    fn listed(v: &[crate::listing::Entry]) -> Vec<(String, bool)> {
        let mut out: Vec<_> = v.iter().map(|e| (e.name.clone(), e.is_dir())).collect();
        out.sort();
        out
    }

    #[test]
    fn split_path_real_dir_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(split_path(d.path()), None);
    }

    #[test]
    fn split_path_root_and_deep() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_fixture(&a);
        assert_eq!(split_path(&a), Some((a.clone(), PathBuf::new())));
        assert_eq!(
            split_path(&a.join("docs/img")),
            Some((a.clone(), "docs/img".into()))
        );
    }

    #[test]
    fn split_path_missing_dir_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(split_path(&d.path().join("gone/deeper")), None);
    }

    #[test]
    fn split_path_non_archive_file_is_none() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("notes.txt"), "x").unwrap();
        fs::write(d.path().join("fake.zip"), "not a zip").unwrap();
        assert_eq!(split_path(&d.path().join("notes.txt/x")), None);
        // A file named like an archive is still taken as one; listing it then fails.
        assert!(split_path(&d.path().join("fake.zip/x")).is_some());
        assert!(list(&d.path().join("fake.zip"), Path::new(""), true).is_err());
    }

    #[test]
    fn list_zip_tar_7z_with_implicit_dirs() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        assert_eq!(
            listed(&list(&z, Path::new(""), true).unwrap()),
            [
                (".hidden".into(), false),
                ("docs".into(), true),
                ("top.txt".into(), false)
            ]
        );
        let inner = list(&z, Path::new("docs"), true).unwrap();
        assert_eq!(
            listed(&inner),
            [("a.txt".into(), false), ("img".into(), true)]
        );
        let a = inner.iter().find(|e| e.name == "a.txt").unwrap();
        assert_eq!((a.size, a.ext.as_str()), (2, "txt"));
        // tar.gz and 7z made by our own packer from a real tree.
        let src = d.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/f"), "123").unwrap();
        for f in [Format::TarGz, Format::SevenZ] {
            let a = d.path().join(format!("t.{}", f.ext()));
            pack(
                f,
                d.path(),
                &[(vec![src.clone()], a.clone())],
                &mut Script::default(),
            );
            assert_eq!(
                listed(&list(&a, Path::new(""), true).unwrap()),
                [("src".into(), true)],
                "{f:?}"
            );
            let sub = list(&a, Path::new("src/sub"), true).unwrap();
            assert_eq!(listed(&sub), [("f".into(), false)], "{f:?}");
            assert_eq!(sub[0].size, 3, "{f:?}");
        }
    }

    #[test]
    fn list_hides_dot_files() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let names = listed(&list(&z, Path::new(""), false).unwrap());
        assert!(names.iter().all(|(n, _)| n != ".hidden"));
    }

    #[test]
    fn list_missing_inner_is_not_found() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        assert_eq!(
            list(&z, Path::new("nope"), true).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            list(&z, Path::new("top.txt"), true).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
    }

    #[test]
    fn list_sees_rewritten_archive() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        assert_eq!(list(&z, Path::new(""), true).unwrap().len(), 3);
        let mut w = zip::ZipWriter::new(fs::File::create(&z).unwrap());
        w.start_file("only.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"longer content so the size differs").unwrap();
        w.finish().unwrap();
        assert_eq!(
            listed(&list(&z, Path::new(""), true).unwrap()),
            [("only.txt".into(), false)]
        );
    }

    fn ex(a: &Path, inner: &str, names: &[&str], dest: &Path, h: &mut Script) -> Report {
        let names: Vec<PathBuf> = names.iter().map(PathBuf::from).collect();
        extract(a, Path::new(inner), &names, dest, h)
    }

    #[test]
    fn extract_one_file() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let out = d.path().join("out");
        let r = ex(&z, "docs", &["a.txt"], &out, &mut Script::default());
        assert_eq!(names(&out), ["a.txt"]);
        assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "aa");
        assert_eq!(r.completed, [z.join("docs/a.txt")]);
    }

    #[test]
    fn extract_dir_subtree() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let out = d.path().join("out");
        ex(&z, "", &["docs"], &out, &mut Script::default());
        assert_eq!(names(&out), ["docs"]);
        assert_eq!(
            fs::read_to_string(out.join("docs/img/p.png")).unwrap(),
            "png"
        );
    }

    #[test]
    fn extract_two_names() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let out = d.path().join("out");
        ex(
            &z,
            "",
            &["top.txt", ".hidden"],
            &out,
            &mut Script::default(),
        );
        assert_eq!(names(&out), [".hidden", "top.txt"]);
    }

    #[test]
    fn extract_conflict_skip() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("top.txt"), "mine").unwrap();
        let mut h = Script {
            conflicts: vec![Resolution::Skip],
            ..Default::default()
        };
        ex(&z, "", &["top.txt"], &out, &mut h);
        assert_eq!(fs::read_to_string(out.join("top.txt")).unwrap(), "mine");
        assert_eq!(names(&out), ["top.txt"]);
    }

    #[test]
    fn extract_cancel_leaves_no_staging() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        zip_fixture(&z);
        let out = d.path().join("out");
        let r = ex(
            &z,
            "",
            &["docs"],
            &out,
            &mut Script {
                cancel: true,
                ..Default::default()
            },
        );
        assert!(r.cancelled);
        assert!(names(&out).is_empty());
    }

    #[test]
    fn extract_selected_evil_entry_stays_inside() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let a = d.path().join("e.tar");
        evil_tar(
            &a,
            &[
                (
                    "sel/ln",
                    tar::EntryType::Symlink,
                    outside.to_str().unwrap(),
                    b"",
                ),
                ("sel/ln/pwned", tar::EntryType::Regular, "", b"x"),
                ("sel/ok", tar::EntryType::Regular, "", b"1"),
                ("other", tar::EntryType::Regular, "", b"2"),
            ],
        );
        let out = d.path().join("out");
        let mut h = Script {
            errors: vec![ErrorChoice::Skip],
            ..Default::default()
        };
        ex(&a, "", &["sel"], &out, &mut h);
        assert!(!outside.join("pwned").exists());
        assert_eq!(names(&out), ["sel"]);
        assert_eq!(fs::read_to_string(out.join("sel/ok")).unwrap(), "1");
    }

    #[test]
    fn fresh_temp_dirs_differ() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (
            fresh_temp_dir(d.path()).unwrap(),
            fresh_temp_dir(d.path()).unwrap(),
        );
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        assert!(a.starts_with(d.path().join(std::process::id().to_string())));
    }

    #[test]
    fn clean_temp_removes_dead_pids_only() {
        let d = tempfile::tempdir().unwrap();
        let mine = fresh_temp_dir(d.path()).unwrap();
        let dead = d.path().join("999999999/0");
        fs::create_dir_all(&dead).unwrap();
        fs::create_dir_all(d.path().join("not-a-pid")).unwrap();
        clean_temp(d.path());
        assert!(mine.is_dir());
        assert!(!d.path().join("999999999").exists());
        assert!(d.path().join("not-a-pid").exists());
    }

    #[test]
    fn regression_split_path_trailing_slash() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_fixture(&a);
        let slashed = PathBuf::from(format!("{}/", a.display()));
        assert_eq!(split_path(&slashed), Some((a.clone(), PathBuf::new())));
        assert_eq!(crate::listing::scan(&slashed, true).unwrap().len(), 3);
    }

    #[test]
    fn temp_root_is_on_disk_not_in_the_runtime_tmpfs() {
        let home = Path::new("/home/u");
        assert_eq!(
            temp_root_in(Some("/c".into()), home),
            Path::new("/c/shagoff-commander")
        );
        assert_eq!(
            temp_root_in(None, home),
            Path::new("/home/u/.cache/shagoff-commander")
        );
        assert_eq!(
            temp_root_in(Some("".into()), home),
            Path::new("/home/u/.cache/shagoff-commander")
        );
    }
}
