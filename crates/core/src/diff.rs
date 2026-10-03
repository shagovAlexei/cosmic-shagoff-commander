//! Ctrl+Shift+D: compare two files line by line (side by side), or byte by byte when binary.

use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

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
pub enum Outcome {
    /// `blocks`: index of the first row of every run of differences.
    Text {
        rows: Vec<Row>,
        blocks: Vec<usize>,
    },
    Binary {
        same: bool,
    },
}

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
        let mut f = File::open(p)?;
        let len = f.metadata()?.len();
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
        (Ok(ta), Ok(tb)) => {
            let (rows, blocks) = rows(&ta, &tb);
            Ok(Outcome::Text { rows, blocks })
        }
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

/// Aligned rows and the start of every block of differences. A replaced block pairs its lines
/// one to one (`Changed`); the longer side's rest is `Deleted` / `Inserted`.
pub fn rows(a: &str, b: &str) -> (Vec<Row>, Vec<usize>) {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let left = |i: usize| Some((i + 1, la[i].to_string()));
    let right = |i: usize| Some((i + 1, lb[i].to_string()));
    let mut out = Vec::new();
    let mut blocks = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &la, &lb) {
        if !matches!(op, DiffOp::Equal { .. })
            && out.last().is_none_or(|r: &Row| r.kind == Kind::Same)
        {
            blocks.push(out.len());
        }
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    out.push(Row {
                        kind: Kind::Same,
                        left: left(old_index + k),
                        right: right(new_index + k),
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for k in 0..old_len {
                    out.push(Row {
                        kind: Kind::Deleted,
                        left: left(old_index + k),
                        right: None,
                    });
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for k in 0..new_len {
                    out.push(Row {
                        kind: Kind::Inserted,
                        left: None,
                        right: right(new_index + k),
                    });
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for k in 0..old_len.max(new_len) {
                    let (l, r) = (
                        (k < old_len).then(|| old_index + k),
                        (k < new_len).then(|| new_index + k),
                    );
                    let kind = match (l, r) {
                        (Some(_), Some(_)) => Kind::Changed,
                        (Some(_), None) => Kind::Deleted,
                        _ => Kind::Inserted,
                    };
                    out.push(Row {
                        kind,
                        left: l.and_then(left),
                        right: r.and_then(right),
                    });
                }
            }
        }
    }
    (out, blocks)
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
        let (rows, blocks) = rows("a\nb\n", "a\nb\n");
        assert_eq!(kinds(&rows), [Kind::Same, Kind::Same]);
        assert!(blocks.is_empty());
        assert_eq!(rows[1].left, Some((2, "b".into())));
    }

    #[test]
    fn insert_delete_change() {
        let (r, _) = rows("a\nb\nc\n", "a\nc\nd\n");
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
        let (r, _) = rows("x\n1\n2\n3\ny\n", "x\nA\nB\ny\n");
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
        let (_, b) = rows("a\nb\nc\nd\ne\n", "a\nB\nc\nd\nE\n");
        assert_eq!(b, [1, 4]);
    }

    #[test]
    fn empty_vs_text() {
        let (r, b) = rows("", "a\nb\n");
        assert_eq!(kinds(&r), [Kind::Inserted, Kind::Inserted]);
        assert_eq!(b, [0]);
    }

    #[test]
    fn no_trailing_newline() {
        let (r, b) = rows("a\nb", "a\nb\n");
        assert!(b.is_empty(), "{:?}", kinds(&r));
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
        let Outcome::Text { rows, blocks } = compare(&a, &b).unwrap() else {
            panic!("not text")
        };
        assert_eq!(kinds(&rows), [Kind::Same, Kind::Changed]);
        assert_eq!(blocks, [1]);
    }

    #[test]
    fn large_is_binary() {
        assert!(is_binary(b"abc", TEXT_LIMIT + 1));
        assert!(!is_binary(b"abc", 3));
        assert!(is_binary(&[0xff, 0xfe], 2)); // not UTF-8
    }
}
