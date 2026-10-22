//! Files → «Split file» / «Combine files» (TC `cm_Split`, `cm_Combine`): `name.001`, `name.002`, …
//! of a given size, and `name.crc` as TC writes it (`filename=`, `size=`, `crc32=`), so parts
//! made by either one are joined by the other.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// A part size as typed: bytes, or with K / M / G (KB, MB, GB, any case, 1024-based), a decimal
/// point allowed (`1.44M`, `700 MB`, `4,7G`). Zero or nonsense: `None`.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().replace(',', ".").to_lowercase();
    let s = s.strip_suffix('b').unwrap_or(&s).trim_end();
    let (num, unit) = match s.chars().last()? {
        'k' => (&s[..s.len() - 1], 1u64 << 10),
        'm' => (&s[..s.len() - 1], 1 << 20),
        'g' => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let n: f64 = num.trim().parse().ok()?;
    let bytes = (n * unit as f64).round();
    (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
}

/// Part `i` (from 1) of `name`: `name.001`; more digits past 999, as TC.
pub fn part_name(name: &OsStr, i: usize) -> OsString {
    let mut n = name.to_os_string();
    n.push(format!(".{i:03}"));
    n
}

/// Cuts `src` into parts of `size` bytes in `dir`, then writes `name.crc`; nothing is written
/// over (the first one there stops it, before any part). On an error the parts written go.
// ponytail: on the caller's thread with no progress or cancel; a progress bar wants `jobs`.
pub fn split(src: &Path, dir: &Path, size: u64) -> io::Result<Vec<PathBuf>> {
    let name = src.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let total = fs::metadata(src)?.len();
    let count = total.div_ceil(size).max(1) as usize;
    let crc_file = dir.join(crc_name(name));
    let parts: Vec<PathBuf> = (1..=count).map(|i| dir.join(part_name(name, i))).collect();
    if let Some(there) = parts
        .iter()
        .chain([&crc_file])
        .find(|p| p.symlink_metadata().is_ok())
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            there.display().to_string(),
        ));
    }
    let mut made = Vec::new();
    let result = (|| {
        let mut from = fs::File::open(src)?;
        let mut crc = crc32fast::Hasher::new();
        for p in &parts {
            let mut to = fs::File::create_new(p)?;
            made.push(p.clone());
            copy_n(&mut from, &mut to, size, &mut crc)?;
        }
        let line = format!(
            "filename={}\r\nsize={total}\r\ncrc32={:08X}\r\n",
            name.to_string_lossy(),
            crc.finalize()
        );
        fs::File::create_new(&crc_file)?.write_all(line.as_bytes())?;
        Ok(())
    })();
    if let Err(e) = result {
        made.iter().for_each(|p| drop(fs::remove_file(p)));
        return Err(e);
    }
    Ok(parts)
}

/// The parts `one` (any of them, or the `.crc`) belongs to: `base.001`, `base.002`, … while they
/// are there, and the name to join them as (the `.crc`'s `filename=`, else `base`'s name).
/// Not a part: `None`.
pub fn parts_of(one: &Path) -> Option<(OsString, Vec<PathBuf>)> {
    let name = one.file_name()?.to_str()?;
    let (base, ext) = name.rsplit_once('.')?;
    let digits = ext.len() >= 3 && ext.bytes().all(|b| b.is_ascii_digit());
    if !(digits || ext.eq_ignore_ascii_case("crc")) {
        return None;
    }
    let dir = one.parent()?;
    let parts: Vec<PathBuf> = (1..)
        .map(|i| dir.join(part_name(OsStr::new(base), i)))
        .take_while(|p| p.is_file())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let info = crc_info(&dir.join(crc_name(OsStr::new(base))));
    let joined = (info.as_ref())
        .and_then(|i| i.name.clone())
        .unwrap_or_else(|| base.into());
    Some((joined, parts))
}

/// Joins `parts` into `dir/name` (never over a file); with a `.crc` next to the first, checks
/// the size and CRC: a mismatch is an error, the joined file left for a look.
pub fn combine(parts: &[PathBuf], dir: &Path, name: &OsStr) -> io::Result<PathBuf> {
    let to_path = dir.join(name);
    let mut to = fs::File::create_new(&to_path)?;
    let mut crc = crc32fast::Hasher::new();
    let mut total = 0;
    let joined = (|| {
        for p in parts {
            total += copy_n(&mut fs::File::open(p)?, &mut to, u64::MAX, &mut crc)?;
        }
        to.flush()
    })();
    if let Err(e) = joined {
        drop(to);
        let _ = fs::remove_file(&to_path);
        return Err(e);
    }
    let base = parts
        .first()
        .and_then(|p| p.file_stem())
        .unwrap_or_default();
    let dir_of = parts.first().and_then(|p| p.parent()).unwrap_or(dir);
    if let Some(info) = crc_info(&dir_of.join(crc_name(base))) {
        let crc = crc.finalize();
        if info.size.is_some_and(|s| s != total) || info.crc.is_some_and(|c| c != crc) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, CRC_MISMATCH));
        }
    }
    Ok(to_path)
}

/// `combine`'s error when the joined file is not the one the `.crc` describes.
pub const CRC_MISMATCH: &str = "crc mismatch";

fn crc_name(name: &OsStr) -> OsString {
    let mut n = name.to_os_string();
    n.push(".crc");
    n
}

struct CrcInfo {
    /// Only a plain name is taken: a path in it could put the file anywhere.
    name: Option<OsString>,
    size: Option<u64>,
    crc: Option<u32>,
}

fn crc_info(file: &Path) -> Option<CrcInfo> {
    let text = fs::read(file).ok()?;
    let text = String::from_utf8_lossy(&text);
    let field = |key: &str| {
        (text.lines())
            .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    let name = field("filename")
        .filter(|n| !n.is_empty() && *n != "." && *n != ".." && !n.contains(['/', '\\']))
        .map(OsString::from);
    Some(CrcInfo {
        name,
        size: field("size").and_then(|s| s.parse().ok()),
        crc: field("crc32").and_then(|s| u32::from_str_radix(s, 16).ok()),
    })
}

/// Up to `n` bytes `from` → `to`, through `crc`; how many.
fn copy_n(
    from: &mut impl Read,
    to: &mut impl Write,
    n: u64,
    crc: &mut crc32fast::Hasher,
) -> io::Result<u64> {
    let mut buf = vec![0; 1 << 16];
    let mut done = 0;
    while done < n {
        let want = buf.len().min((n - done).min(usize::MAX as u64) as usize);
        let got = from.read(&mut buf[..want])?;
        if got == 0 {
            break;
        }
        crc.update(&buf[..got]);
        to.write_all(&buf[..got])?;
        done += got as u64;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_read_as_typed() {
        assert_eq!(parse_size("1000"), Some(1000));
        assert_eq!(parse_size("100K"), Some(102_400));
        assert_eq!(parse_size("1.5 mb"), Some(1_572_864));
        assert_eq!(
            parse_size("4,7G"),
            Some((4.7 * (1u64 << 30) as f64).round() as u64)
        );
        for bad in ["", "0", "x", "-5M", "K"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_file_is_cut_and_joined_back_checked_by_its_crc() {
        let d = tempfile::tempdir().unwrap();
        let (src, parts_dir, out) = (
            d.path().join("big.bin"),
            d.path().join("p"),
            d.path().join("o"),
        );
        fs::create_dir(&parts_dir).unwrap();
        fs::create_dir(&out).unwrap();
        let data: Vec<u8> = (0..2500u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let parts = split(&src, &parts_dir, 1000).unwrap();
        let names: Vec<_> = parts
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["big.bin.001", "big.bin.002", "big.bin.003"]);
        assert_eq!(fs::metadata(&parts[2]).unwrap().len(), 500);
        let crc = fs::read_to_string(parts_dir.join("big.bin.crc")).unwrap();
        assert!(crc.starts_with("filename=big.bin\r\nsize=2500\r\ncrc32="));
        // Again: the parts are there, nothing is written.
        assert!(split(&src, &parts_dir, 1000).is_err());
        // From any part, or the .crc: all of them, under the original name.
        let (name, found) = parts_of(&parts_dir.join("big.bin.002")).unwrap();
        assert_eq!((name.as_os_str(), found.len()), (OsStr::new("big.bin"), 3));
        assert!(parts_of(&parts_dir.join("big.bin.crc")).is_some());
        assert!(parts_of(&src).is_none());
        let joined = combine(&found, &out, &name).unwrap();
        assert_eq!(fs::read(&joined).unwrap(), data);
        assert!(combine(&found, &out, &name).is_err()); // there: kept
        // A damaged part: the CRC says so.
        fs::write(&parts[1], vec![0u8; 1000]).unwrap();
        let err = combine(&found, &out, OsStr::new("again.bin")).unwrap_err();
        assert_eq!(err.to_string(), CRC_MISMATCH);
        // A .crc naming a path is not followed.
        fs::write(parts_dir.join("big.bin.crc"), "filename=../evil\r\n").unwrap();
        assert_eq!(parts_of(&parts[0]).unwrap().0, OsStr::new("big.bin"));
    }
}
