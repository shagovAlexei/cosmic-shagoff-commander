//! Fuzz-style smoke test (207c): mutated archives and random text through the parsers; the only
//! thing checked is that nothing panics or hangs. Fixed seeds, so a failure repeats. A real
//! `cargo fuzz` needs nightly and is not set up.

use shagoff_core::ops::{ErrorChoice, FileInfo, Handler, Resolution};
use shagoff_core::{archive, cmdline, lister, mask, thumbcache};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

struct Quiet;
impl Handler for Quiet {
    fn progress(&mut self, _: u64, _: u64, _: &Path) {}
    fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
        Resolution::Skip
    }
    fn error(&mut self, _: &Path, _: &io::Error) -> ErrorChoice {
        ErrorChoice::Skip
    }
    fn cancelled(&self) -> bool {
        false
    }
}

fn mutate(r: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..1 + r.below(8) {
        match r.below(4) {
            0 if !b.is_empty() => {
                let i = r.below(b.len());
                b[i] = r.next() as u8;
            }
            1 if !b.is_empty() => {
                let i = r.below(b.len());
                b[i] ^= 1 << r.below(8);
            }
            2 => b.truncate(r.below(b.len() + 1)),
            _ => {
                let i = r.below(b.len() + 1);
                b.insert(i, r.next() as u8);
            }
        }
    }
    b
}

fn text(r: &mut Rng) -> String {
    const PIECES: &[&str] = &[
        "*", "?", ".", "..", "/", "~", "%", "%P", "%S", "%N", "\"", "'", "\\", " ", ";", "|", "&",
        "cd ", "a", "txt", "[", "]", "{", "}", "(", ")", "é", "я", "\u{0}", "\n", "**/", "-",
    ];
    (0..r.below(24))
        .map(|_| PIECES[r.below(PIECES.len())])
        .collect()
}

#[test]
fn archives_survive_garbage() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), "hello").unwrap();
    fs::write(src.join("sub/b.bin"), [0u8; 300]).unwrap();
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let kinds = [
        (archive::Format::Zip, "z.zip"),
        (archive::Format::Tar, "t.tar"),
        (archive::Format::TarGz, "t.tar.gz"),
        (archive::Format::SevenZ, "s.7z"),
    ];
    for (f, name) in kinds {
        let good = d.path().join(name);
        archive::pack(
            f,
            &src,
            &[(vec![src.join("a.txt"), src.join("sub")], good.clone())],
            &mut Quiet,
        );
        let seed = fs::read(&good).unwrap();
        assert!(!seed.is_empty(), "{name} not packed");
        for n in 0..60 {
            let dir = d.path().join(format!("{name}.{n}"));
            fs::create_dir_all(&dir).unwrap();
            let bad = dir.join(name);
            fs::write(&bad, mutate(&mut r, &seed)).unwrap();
            let _ = archive::list(&bad, Path::new(""), true);
            let _ = archive::list(&bad, Path::new("sub"), true);
            archive::unpack(&[bad], &dir.join("out"), false, &mut Quiet);
        }
    }
}

#[test]
fn text_parsers_survive_garbage() {
    let mut r = Rng(0xD1B5_4A32_D192_ED03);
    let (cwd, home) = (PathBuf::from("/tmp"), PathBuf::from("/home/x"));
    for _ in 0..5000 {
        let s = text(&mut r);
        let m = mask::Mask::parse(&s);
        let _ = (m.matches(&text(&mut r)), m.skips_dir(&s), m.is_all());
        let _ = cmdline::parse(&s, &cwd, &home);
        let _ = cmdline::argv(&s, &cwd, None);
        let _ = cmdline::resolve(&s, &cwd, &home);
        let _ = cmdline::quote(&s);
        let sel = [text(&mut r)];
        let v = cmdline::Vars {
            dir: &cwd,
            other: &home,
            name: Some(&s),
            selected: &sel,
        };
        let _ = cmdline::expand(&s, &v);
    }
}

#[test]
fn byte_parsers_survive_garbage() {
    let mut r = Rng(0x2545_F491_4F6C_DD1D);
    let d = tempfile::tempdir().unwrap();
    for n in 0..2000 {
        let len = r.below(600);
        let b: Vec<u8> = (0..len).map(|_| r.next() as u8).collect();
        let _ = lister::decode(&b);
        let enc = lister::guess(&b);
        let _ = lister::decode_as(&b, enc);
        let _ = lister::lines(&b, 1 + r.below(120), n % 2 == 0);
        let _ = lister::detect("f.bin", &b);
        let _ = lister::hex_row(&b, 0, 0);
        let _ = lister::find_bytes(&b, &b[..len.min(3)], r.below(len + 1), n % 2 == 0);
        if n % 20 == 0 {
            let p = d.path().join("c.png");
            fs::write(&p, &b).unwrap();
            let _ = thumbcache::read(&p, "file:///x", 1, 1);
        }
    }
}
