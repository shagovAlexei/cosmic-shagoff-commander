//! Ctrl+Shift+D: compare two files line by line (side by side), or byte by byte when binary.

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
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
    /// The two texts compared (for re-comparing with other options and copying blocks).
    pub src: Arc<(String, String)>,
}

/// What counts as equal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    /// Runs of spaces / tabs count as one, and leading / trailing ones not at all.
    pub ignore_space: bool,
    pub ignore_case: bool,
}

impl Opts {
    fn key(self, line: &str) -> String {
        let s = if self.ignore_space {
            line.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            line.to_string()
        };
        if self.ignore_case {
            s.to_lowercase()
        } else {
            s
        }
    }
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
    compare_with(a, b, Opts::default())
}

/// `compare` with lines compared by `o`.
pub fn compare_with(a: &Path, b: &Path, o: Opts) -> io::Result<Outcome> {
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
        (Ok(ta), Ok(tb)) => Ok(Outcome::Text(rows_with(&ta, &tb, ROWS_LIMIT, o))),
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
    rows_with(a, b, limit, Opts::default())
}

/// `rows` comparing lines by `o` (shown as they are written).
pub fn rows_with(a: &str, b: &str, limit: usize, o: Opts) -> Text {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let deadline = Some(Instant::now() + DEADLINE);
    let ops = if o == Opts::default() {
        capture_diff_slices_deadline(Algorithm::Myers, &la, &lb, deadline)
    } else {
        let key = |v: &[&str]| v.iter().map(|l| o.key(l)).collect::<Vec<_>>();
        capture_diff_slices_deadline(Algorithm::Myers, &key(&la), &key(&lb), deadline)
    };
    let mut t = Text {
        rows: Vec::new(),
        blocks: Vec::new(),
        total: 0,
        eol_differs: false,
        src: Arc::new((a.to_string(), b.to_string())),
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
    // With options, lines equal "enough" are not an end-of-line difference.
    t.eol_differs = t.blocks.is_empty() && a != b && o == Opts::default();
    t
}

/// The differing middle of a changed line pair: byte ranges after the common prefix and before
/// the common suffix (on char boundaries; they never overlap).
pub fn inline(a: &str, b: &str) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let mut p = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            break;
        }
        p += x.len_utf8();
    }
    let mut q = 0;
    for (x, y) in a[p..].chars().rev().zip(b[p..].chars().rev()) {
        if x != y {
            break;
        }
        q += x.len_utf8();
    }
    (p..a.len() - q, p..b.len() - q)
}

/// Copy difference block `block` to the other side (`to_right`: left → right): the target's
/// lines of that block are replaced by the source's. The new target text, keeping its line
/// endings (CRLF / LF) and whether it ends with one. `None`: no such block.
pub fn copy_block(a: &str, b: &str, o: Opts, block: usize, to_right: bool) -> Option<String> {
    let t = rows_with(a, b, usize::MAX, o);
    let start = *t.blocks.get(block)?;
    let end = t.rows[start..]
        .iter()
        .position(|r| r.kind == Kind::Same)
        .map_or(t.rows.len(), |k| start + k);
    let side = |r: &Row| {
        if to_right {
            (r.left.clone(), r.right.clone())
        } else {
            (r.right.clone(), r.left.clone())
        }
    };
    let (src_text, dst_text) = if to_right { (a, b) } else { (b, a) };
    let block_rows = &t.rows[start..end];
    let src: Vec<String> = block_rows
        .iter()
        .filter_map(|r| side(r).0)
        .map(|x| x.1)
        .collect();
    let nums: Vec<usize> = block_rows
        .iter()
        .filter_map(|r| side(r).1)
        .map(|x| x.0)
        .collect();
    let (r0, r1) = match (nums.first(), nums.last()) {
        (Some(&f), Some(&l)) => (f - 1, l),
        _ => {
            // Nothing of the block on the target: insert where the next (or after the previous)
            // target line is.
            let at = t.rows[end..]
                .iter()
                .find_map(|r| side(r).1.map(|x| x.0 - 1))
                .or_else(|| {
                    t.rows[..start]
                        .iter()
                        .rev()
                        .find_map(|r| side(r).1.map(|x| x.0))
                })
                .unwrap_or(0);
            (at, at)
        }
    };
    let dst: Vec<&str> = dst_text.lines().collect();
    let style = if dst_text.is_empty() {
        src_text
    } else {
        dst_text
    };
    let eol = if style.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = dst[..r0]
        .iter()
        .copied()
        .chain(src.iter().map(String::as_str))
        .chain(dst[r1..].iter().copied())
        .collect();
    let mut out = lines.join(eol);
    if style.ends_with('\n') && !out.is_empty() {
        out.push_str(eol);
    }
    Some(out)
}

/// Write `text` to `path` through a part file and a rename (a crash never leaves half a file),
/// keeping the file's permissions.
pub fn save(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let perms = fs::metadata(path)?.permissions();
    let (part, mut f) = crate::ops::create_part(path)?;
    let done = f
        .write_all(text.as_bytes())
        .and_then(|()| f.sync_all())
        .and_then(|()| fs::set_permissions(&part, perms))
        .and_then(|()| fs::rename(&part, path));
    if done.is_err() {
        let _ = fs::remove_file(&part);
    }
    done
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

    #[test]
    fn ignore_space_and_case() {
        let (a, b) = ("Hello  world\nx\n", "hello world \nx\n");
        assert_eq!(rows(a, b, 100).blocks, [0]);
        let o = Opts {
            ignore_space: true,
            ignore_case: true,
        };
        let t = rows_with(a, b, 100, o);
        assert!(t.blocks.is_empty() && !t.eol_differs);
        // Shown as written, not normalized.
        assert_eq!(t.rows[0].left, Some((1, "Hello  world".into())));
        let only_space = Opts {
            ignore_space: true,
            ..Opts::default()
        };
        assert_eq!(rows_with(a, b, 100, only_space).blocks, [0]); // case still differs
    }

    #[test]
    fn inline_marks_the_differing_middle() {
        assert_eq!(inline("let x = 1;", "let y = 1;"), (4..5, 4..5));
        assert_eq!(inline("abc", "abXYc"), (2..2, 2..4));
        assert_eq!(inline("привет", "превет"), (4..6, 4..6)); // on char boundaries
        assert_eq!(inline("same", "same"), (4..4, 4..4));
        assert_eq!(inline("aa", "aaa"), (2..2, 2..3)); // prefix and suffix never overlap
    }

    #[test]
    fn copy_block_both_ways() {
        let (a, b) = ("x\n1\n2\ny\nz\n", "x\nA\ny\nz\nw\n");
        let o = Opts::default();
        // Block 0 (1,2 vs A) left → right.
        assert_eq!(copy_block(a, b, o, 0, true).unwrap(), "x\n1\n2\ny\nz\nw\n");
        // Block 1 (w only on the right) right → left: inserted at the end.
        assert_eq!(copy_block(a, b, o, 1, false).unwrap(), "x\n1\n2\ny\nz\nw\n");
        // left → right of a right-only block deletes it there.
        assert_eq!(copy_block(a, b, o, 1, true).unwrap(), "x\nA\ny\nz\n");
        assert_eq!(copy_block(a, b, o, 5, true), None);
    }

    #[test]
    fn copy_block_keeps_the_targets_line_endings() {
        let (a, b) = ("a\nB\nc\n", "a\r\nb\r\nc");
        assert_eq!(
            copy_block(a, b, Opts::default(), 0, true).unwrap(),
            "a\r\nB\r\nc"
        );
    }

    #[test]
    fn save_replaces_the_file_keeping_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.sh");
        fs::write(&f, "old").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o751)).unwrap();
        save(&f, "new").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o751
        );
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1); // no part file left
    }
}
