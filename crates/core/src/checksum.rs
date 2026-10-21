//! Files → «Create checksum files» / «Verify checksums» (TC `cm_CRCcreate` / `cm_CRCcheck`): lines
//! of `hash *name` as `md5sum` / `sha256sum` write and read them (`-c` takes ours too).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::Digest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    pub const ALL: [Algo; 4] = [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512];

    pub fn ext(self) -> &'static str {
        match self {
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
            Algo::Sha256 => "sha256",
            Algo::Sha512 => "sha512",
        }
    }

    /// By its hex length: one file may mix them, and `SHA256SUMS` has no extension to go by.
    fn by_len(n: usize) -> Option<Algo> {
        Algo::ALL.into_iter().find(|a| a.hex_len() == n)
    }

    fn hex_len(self) -> usize {
        match self {
            Algo::Md5 => 32,
            Algo::Sha1 => 40,
            Algo::Sha256 => 64,
            Algo::Sha512 => 128,
        }
    }
}

/// A checksum file by its name: `.md5`, `.sha1`, `.sha256`, `.sha512`, `.sha`, or `…SUMS`.
pub fn is_sum_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.ends_with("SUMS")
        || (Algo::ALL.iter().map(|a| a.ext()).chain(["sha"]))
            .any(|e| lower.strip_suffix(e).is_some_and(|s| s.ends_with('.')))
}

/// `path`'s hash in lower-case hex. Stopped: `Interrupted`.
pub fn hash(path: &Path, algo: Algo, stop: &AtomicBool) -> io::Result<String> {
    fn run<D: Digest>(mut r: impl Read, stop: &AtomicBool) -> io::Result<String> {
        let mut d = D::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            match r.read(&mut buf)? {
                0 => break,
                n => d.update(&buf[..n]),
            }
        }
        Ok(d.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
    let f = File::open(path)?;
    match algo {
        Algo::Md5 => run::<md5::Md5>(f, stop),
        Algo::Sha1 => run::<sha1::Sha1>(f, stop),
        Algo::Sha256 => run::<sha2::Sha256>(f, stop),
        Algo::Sha512 => run::<sha2::Sha512>(f, stop),
    }
}

/// The files to sum for `targets` in `base`: files as they are, dirs walked (links never
/// followed, special files left out), each with its name relative to `base`.
fn files(base: &Path, targets: &[PathBuf], stop: &AtomicBool) -> io::Result<Vec<PathBuf>> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>, stop: &AtomicBool) -> io::Result<()> {
        if stop.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let t = fs::symlink_metadata(p)?.file_type();
        if t.is_dir() {
            let mut kids: Vec<_> = fs::read_dir(p)?
                .map(|e| e.map(|e| e.path()))
                .collect::<io::Result<_>>()?;
            kids.sort();
            for k in kids {
                walk(&k, out, stop)?;
            }
        } else if t.is_file() {
            out.push(p.to_path_buf());
        }
        Ok(())
    }
    let mut out = Vec::new();
    for t in targets {
        walk(&base.join(t), &mut out, stop)?;
    }
    Ok(out
        .into_iter()
        .map(|p| p.strip_prefix(base).map(Path::to_path_buf).unwrap_or(p))
        .collect())
}

/// Sums `targets` (in `dir`) into checksum files there: one per file (`a.txt.sha256`; dirs
/// left out) with `separate`, else one named after `dir` (`docs.sha256`). Written whole then
/// renamed; an existing file is never replaced (`AlreadyExists`). Returns the files written.
pub fn create(
    dir: &Path,
    targets: &[PathBuf],
    algo: Algo,
    separate: bool,
    stop: &AtomicBool,
) -> io::Result<Vec<PathBuf>> {
    let all = files(dir, targets, stop)?;
    let groups: Vec<(PathBuf, Vec<PathBuf>)> = if separate {
        (all.into_iter())
            .filter(|f| f.parent() == Some(Path::new("")))
            .map(|f| {
                let mut name = f.clone().into_os_string();
                name.push(format!(".{}", algo.ext()));
                (dir.join(name), vec![f])
            })
            .collect()
    } else {
        let stem = dir
            .file_name()
            .map_or("checksums".into(), |n| n.to_os_string());
        let mut name = stem;
        name.push(format!(".{}", algo.ext()));
        vec![(dir.join(name), all)]
    };
    let mut written = Vec::new();
    for (out, members) in groups {
        if fs::symlink_metadata(&out).is_ok() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let mut text = Vec::new();
        for m in &members {
            text.extend_from_slice(hash(&dir.join(m), algo, stop)?.as_bytes());
            text.extend_from_slice(b" *");
            text.extend_from_slice(m.as_os_str().as_bytes());
            text.push(b'\n');
        }
        // ponytail: a name with a newline is written as is (GNU escapes it with `\`); rare enough.
        let mut part = out.clone().into_os_string();
        part.push(".shagoff-part");
        let part = PathBuf::from(part);
        let mut f = File::create_new(&part)?;
        let done = f.write_all(&text).and_then(|_| f.sync_all());
        // No replace: someone may have made it meanwhile.
        let done = done.and_then(|_| crate::ops::rename_noreplace(&part, &out));
        if let Err(e) = done {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        written.push(out);
    }
    Ok(written)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    Ok,
    Bad,
    /// Unreadable: why.
    Missing(io::ErrorKind),
}

/// `(hash, name)` of each line `hash *name` / `hash  name` with a known hash length; others
/// (comments, blanks) left out.
pub fn parse(text: &[u8]) -> Vec<(String, PathBuf)> {
    (text.split(|&b| b == b'\n'))
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let sp = line.iter().position(|&b| b == b' ')?;
            let (h, rest) = line.split_at(sp);
            let h = std::str::from_utf8(h).ok()?.trim_start_matches('\\');
            Algo::by_len(h.len()).filter(|_| h.bytes().all(|b| b.is_ascii_hexdigit()))?;
            let name = rest
                .strip_prefix(b" *")
                .or_else(|| rest.strip_prefix(b"  "))?;
            (!name.is_empty()).then(|| {
                (
                    h.to_lowercase(),
                    PathBuf::from(std::ffi::OsString::from_vec(name.to_vec())),
                )
            })
        })
        .collect()
}

/// Each file named in a sum file and what its check found.
pub type Report = Vec<(PathBuf, Check)>;

/// Each file of checksum file `sum` (names relative to its dir) hashed and compared.
pub fn verify(sum: &Path, stop: &AtomicBool) -> io::Result<Report> {
    let base = sum.parent().unwrap_or(Path::new("."));
    let lines = parse(&fs::read(sum)?);
    if lines.is_empty() {
        return Err(io::Error::other(crate::why::NO_SUMS));
    }
    let mut out = Vec::new();
    for (h, name) in lines {
        let algo = Algo::by_len(h.len()).expect("parse keeps known lengths");
        let check = match hash(&base.join(&name), algo, stop) {
            Ok(got) if got == h => Check::Ok,
            Ok(_) => Check::Bad,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            Err(e) => Check::Missing(e.kind()),
        };
        out.push((name, check));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hashes_known_values() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("abc");
        fs::write(&f, "abc").unwrap();
        let no = AtomicBool::new(false);
        assert_eq!(hash(&f, Algo::Md5, &no).unwrap(), ABC_MD5);
        assert_eq!(
            hash(&f, Algo::Sha1, &no).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hash(&f, Algo::Sha256, &no).unwrap(), ABC_SHA256);
        assert_eq!(hash(&f, Algo::Sha512, &no).unwrap().len(), 128);
        let stop = AtomicBool::new(true);
        assert_eq!(
            hash(&f, Algo::Md5, &stop).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn sum_files_by_name() {
        for n in ["a.md5", "a.SHA256", "x.sha", "SHA256SUMS", "b.sha512"] {
            assert!(is_sum_file(n), "{n}");
        }
        for n in ["md5", "a.txt", "sha256", "a.sha2"] {
            assert!(!is_sum_file(n), "{n}");
        }
    }

    #[test]
    fn creates_one_file_with_dirs_walked_and_verifies_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("docs");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("abc"), "abc").unwrap();
        fs::write(dir.join("sub/x"), "abc").unwrap();
        std::os::unix::fs::symlink("abc", dir.join("ln")).unwrap(); // not followed
        let no = AtomicBool::new(false);
        let t = [
            PathBuf::from("abc"),
            PathBuf::from("sub"),
            PathBuf::from("ln"),
        ];
        let out = create(&dir, &t, Algo::Md5, false, &no).unwrap();
        assert_eq!(out, [dir.join("docs.md5")]);
        let text = fs::read_to_string(&out[0]).unwrap();
        assert_eq!(text, format!("{ABC_MD5} *abc\n{ABC_MD5} *sub/x\n"));
        // Never replaced.
        let e = create(&dir, &t, Algo::Md5, false, &no).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(!dir.join("docs.md5.shagoff-part").exists());
        // Verified: one changed, one gone.
        fs::write(dir.join("sub/x"), "abd").unwrap();
        fs::remove_file(dir.join("abc")).unwrap();
        let got = verify(&out[0], &no).unwrap();
        assert_eq!(got[0].0, PathBuf::from("abc"));
        assert!(matches!(got[0].1, Check::Missing(_)));
        assert_eq!(got[1], (PathBuf::from("sub/x"), Check::Bad));
    }

    #[test]
    fn separate_files_skip_dirs() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("abc"), "abc").unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub/x"), "abc").unwrap();
        let no = AtomicBool::new(false);
        let t = [PathBuf::from("abc"), PathBuf::from("sub")];
        let out = create(d.path(), &t, Algo::Sha256, true, &no).unwrap();
        assert_eq!(out, [d.path().join("abc.sha256")]);
        assert_eq!(
            verify(&out[0], &no).unwrap(),
            [(PathBuf::from("abc"), Check::Ok)]
        );
    }

    #[test]
    fn reads_gnu_lines_mixed_and_upper_case() {
        let text = format!(
            "# comment\n{}  a b\r\n{ABC_MD5} *c\nzz *bad\n\n",
            ABC_SHA256.to_uppercase()
        );
        let got = parse(text.as_bytes());
        assert_eq!(
            got,
            [
                (ABC_SHA256.to_string(), PathBuf::from("a b")),
                (ABC_MD5.to_string(), PathBuf::from("c"))
            ]
        );
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("empty.md5");
        fs::write(&f, "nothing here\n").unwrap();
        let no = AtomicBool::new(false);
        assert_eq!(
            verify(&f, &no).unwrap_err().to_string(),
            crate::why::NO_SUMS
        );
    }
}
