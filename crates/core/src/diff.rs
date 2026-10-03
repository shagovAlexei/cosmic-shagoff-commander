//! Ctrl+Shift+D: compare two files line by line (side by side), or byte by byte when binary.

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Same,
    /// Only on the left.
    Deleted,
    /// Only on the right.
    Inserted,
    /// Both sides, different text.
    Changed,
}

/// One aligned line pair: (1-based line number, text) on each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
}

#[derive(Clone, Debug)]
pub struct Text {
    /// The first `limit` aligned rows.
    pub rows: Vec<Row>,
    /// First row of every run of differences (also past `rows`).
    pub blocks: Vec<usize>,
    /// All aligned rows.
    pub total: usize,
    /// No line differs, but the bytes do (CRLF / LF, final newline).
    pub eol_differs: bool,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Text(Text),
    Binary { same: bool },
}

/// Rows kept for the view; the rest are only counted (memory of huge files).
pub const ROWS_LIMIT: usize = 100_000;
/// Myers is quadratic on big rewrites: after this it settles for a coarser (still correct) diff.
const DEADLINE: Duration = Duration::from_secs(5);

/// Bigger files are compared byte by byte only: a side-by-side view of them is useless and huge.
pub const TEXT_LIMIT: u64 = 20 << 20;
const HEAD: usize = 8 << 10;

/// From the first bytes and the size: NUL, invalid UTF-8 or too big.
pub fn is_binary(head: &[u8], len: u64) -> bool {
    len > TEXT_LIMIT
        || head.contains(&0)
        // A char cut at the end of `head` is not an error.
        || std::str::from_utf8(head).is_err_and(|e| e.error_len().is_some())
}

pub fn compare(a: &Path, b: &Path) -> io::Result<Outcome> {
    let head = |p: &Path| -> io::Result<(Vec<u8>, u64)> {
        // A fifo would block `open` forever, a device never ends: regular files only.
        let m = fs::metadata(p)?;
        if !m.is_file() {
            let msg = format!("{}: not a regular file", p.display());
            return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
        }
        let mut f = File::open(p)?;
        let len = m.len();
        let mut buf = vec![0; HEAD];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        Ok((buf, len))
    };
    let ((ha, la), (hb, lb)) = (head(a)?, head(b)?);
    if is_binary(&ha, la) || is_binary(&hb, lb) {
        return Ok(Outcome::Binary {
            same: la == lb && same_bytes(a, b)?,
        });
    }
    match (
        String::from_utf8(fs::read(a)?),
        String::from_utf8(fs::read(b)?),
    ) {
        (Ok(ta), Ok(tb)) => Ok(Outcome::Text(rows(&ta, &tb, ROWS_LIMIT))),
        _ => Ok(Outcome::Binary {
            same: la == lb && same_bytes(a, b)?,
        }),
    }
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
    let (mut ba, mut bb) = (vec![0; 1 << 16], vec![0; 1 << 16]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(fb.read(&mut bb[..1])? == 0);
        }
        if fb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Aligned rows (the first `limit`) and the start of every block of differences. A replaced
/// block pairs its lines one to one (`Changed`); the longer side's rest is `Deleted` / `Inserted`.
pub fn rows(a: &str, b: &str, limit: usize) -> Text {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let deadline = Some(Instant::now() + DEADLINE);
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &la, &lb, deadline);
    let mut t = Text {
        rows: Vec::new(),
        blocks: Vec::new(),
        total: 0,
        eol_differs: false,
    };
    let mut last_same = true;
    let mut push = |t: &mut Text, kind: Kind, l: Option<usize>, r: Option<usize>| {
        if kind != Kind::Same && last_same {
            t.blocks.push(t.total);
        }
        last_same = kind == Kind::Same;
        if t.rows.len() < limit {
            let left = l.map(|i| (i + 1, la[i].to_string()));
            let right = r.map(|i| (i + 1, lb[i].to_string()));
            t.rows.push(Row { kind, left, right });
        }
        t.total += 1;
    };
    for op in ops {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    push(&mut t, Kind::Same, Some(old_index + k), Some(new_index + k));
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for k in 0..old_len {
                    push(&mut t, Kind::Deleted, Some(old_index + k), None);
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for k in 0..new_len {
                    push(&mut t, Kind::Inserted, None, Some(new_index + k));
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for k in 0..old_len.max(new_len) {
                    let l = (k < old_len).then(|| old_index + k);
                    let r = (k < new_len).then(|| new_index + k);
                    let kind = match (l, r) {
                        (Some(_), Some(_)) => Kind::Changed,
                        (Some(_), None) => Kind::Deleted,
                        _ => Kind::Inserted,
                    };
                    push(&mut t, kind, l, r);
                }
            }
        }
    }
    t.eol_differs = t.blocks.is_empty() && a != b;
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn kinds(rows: &[Row]) -> Vec<Kind> {
        rows.iter().map(|r| r.kind).collect()
    }

    #[test]
    fn same_files() {
        let t = rows("a\nb\n", "a\nb\n", 100);
        assert_eq!(kinds(&t.rows), [Kind::Same, Kind::Same]);
        assert!(t.blocks.is_empty() && !t.eol_differs);
        assert_eq!(t.rows[1].left, Some((2, "b".into())));
    }

    #[test]
    fn insert_delete_change() {
        let r = rows("a\nb\nc\n", "a\nc\nd\n", 100).rows;
        assert_eq!(
            kinds(&r),
            [Kind::Same, Kind::Deleted, Kind::Same, Kind::Inserted]
        );
        assert_eq!(r[1].left, Some((2, "b".into())));
        assert_eq!(r[1].right, None);
        assert_eq!(r[3].right, Some((3, "d".into())));
    }

    #[test]
    fn changed_block_pairs() {
        let r = rows("x\n1\n2\n3\ny\n", "x\nA\nB\ny\n", 100).rows;
        assert_eq!(
            kinds(&r),
            [
                Kind::Same,
                Kind::Changed,
                Kind::Changed,
                Kind::Deleted,
                Kind::Same
            ]
        );
        assert_eq!(
            (r[1].left.clone(), r[1].right.clone()),
            (Some((2, "1".into())), Some((2, "A".into())))
        );
        assert_eq!(r[3].left, Some((4, "3".into())));
    }

    #[test]
    fn blocks_mark_runs() {
        let t = rows("a\nb\nc\nd\ne\n", "a\nB\nc\nd\nE\n", 100);
        assert_eq!(t.blocks, [1, 4]);
    }

    #[test]
    fn empty_vs_text() {
        let t = rows("", "a\nb\n", 100);
        assert_eq!(kinds(&t.rows), [Kind::Inserted, Kind::Inserted]);
        assert_eq!(t.blocks, [0]);
    }

    #[test]
    fn no_trailing_newline() {
        let t = rows("a\nb", "a\nb\n", 100);
        assert!(t.blocks.is_empty());
        assert!(t.eol_differs); // not "identical": the bytes differ
    }

    #[test]
    fn binary_same_and_differ() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("b"), d.path().join("c"));
        fs::write(&a, b"\0\x01\x02").unwrap();
        fs::write(&b, b"\0\x01\x02").unwrap();
        fs::write(&c, b"\0\x01\x03").unwrap();
        assert!(matches!(
            compare(&a, &b).unwrap(),
            Outcome::Binary { same: true }
        ));
        assert!(matches!(
            compare(&a, &c).unwrap(),
            Outcome::Binary { same: false }
        ));
    }

    #[test]
    fn text_files_compare() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::write(&a, "привет\nмир\n").unwrap();
        fs::write(&b, "привет\nвсем\n").unwrap();
        let Outcome::Text(t) = compare(&a, &b).unwrap() else {
            panic!("not text")
        };
        assert_eq!(kinds(&t.rows), [Kind::Same, Kind::Changed]);
        assert_eq!(t.blocks, [1]);
    }

    #[test]
    fn large_is_binary() {
        assert!(is_binary(b"abc", TEXT_LIMIT + 1));
        assert!(!is_binary(b"abc", 3));
        assert!(is_binary(&[0xff, 0xfe], 2)); // not UTF-8
    }

    #[test]
    fn regression_crlf_vs_lf_is_not_identical() {
        let t = rows("a\r\nb\r\n", "a\nb\n", 100);
        assert!(t.blocks.is_empty() && t.eol_differs);
    }

    #[test]
    fn regression_rows_are_capped_but_counted() {
        let a: String = (0..50).map(|i| format!("{i}\n")).collect();
        let b: String = (0..50).map(|i| format!("x{i}\n")).collect();
        let t = rows(&a, &b, 10);
        assert_eq!((t.rows.len(), t.total), (10, 50));
        assert_eq!(t.blocks, [0]);
    }

    #[test]
    fn regression_fifo_is_refused_not_read() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("p");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        fs::write(d.path().join("f"), "x").unwrap();
        assert!(compare(&fifo, &d.path().join("f")).is_err());
    }
}
