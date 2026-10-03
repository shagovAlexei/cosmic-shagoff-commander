//! Compare two dirs (Shift+F2 marks, Ctrl+Shift+S sync dialog) and plan the copies.

use crate::listing::Entry;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Same,
    LeftOnly,
    RightOnly,
    LeftNewer,
    RightNewer,
    /// Different, but no side is clearly newer (equal date, other size; file against dir).
    Differ,
}

/// What synchronizing does with a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    None,
    ToRight,
    ToLeft,
    /// Delete the one side that has it (to the trash).
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub size: u64,
    pub mtime: SystemTime,
    pub dir: bool,
    /// A symlink (never followed): its "size" is the target path's length.
    pub link: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub rel: PathBuf,
    pub left: Option<Info>,
    pub right: Option<Info>,
    pub state: State,
    pub dir: Dir,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub recursive: bool,
    /// Equal sizes: compare the bytes.
    pub content: bool,
    /// Size (and content) only.
    pub ignore_date: bool,
    pub hidden: bool,
    /// The right becomes a copy of the left: the left wins every difference, right-only goes.
    pub mirror: bool,
    /// Only files whose name matches (dirs on one side are opened then, not copied whole).
    pub mask: Option<crate::mask::Mask>,
}

/// FAT and zip keep 2-second times: this close is the same time.
const SLACK: Duration = Duration::from_secs(2);

fn same_time(a: SystemTime, b: SystemTime) -> bool {
    let d = match a.duration_since(b) {
        Ok(d) => d,
        Err(e) => e.duration(),
    };
    d <= SLACK
}

pub fn default_dir(s: State) -> Dir {
    match s {
        State::LeftOnly | State::LeftNewer => Dir::ToRight,
        State::RightOnly | State::RightNewer => Dir::ToLeft,
        State::Same | State::Differ => Dir::None,
    }
}

/// Mirror arrows: the left is the source of truth. Different kinds (file / dir / link) get no
/// arrow: replacing one by the other is never automatic.
fn mirror_dir(s: State, a: Option<Info>, b: Option<Info>) -> Dir {
    match (s, a, b) {
        (State::Same, ..) => Dir::None,
        (State::RightOnly, ..) => Dir::Delete,
        (_, Some(a), Some(b)) if a.dir != b.dir || a.link != b.link => Dir::None,
        _ => Dir::ToRight,
    }
}

/// A click on a row's arrow: the next of → ← ✕ (delete, only when one side has it) and none,
/// skipping what the row cannot do (no copy from a side that has nothing).
pub fn next_dir(row: &Row) -> Dir {
    let can = |d: Dir| match d {
        Dir::ToRight => row.left.is_some(),
        Dir::ToLeft => row.right.is_some(),
        Dir::Delete => row.left.is_some() != row.right.is_some(),
        Dir::None => true,
    };
    let order = [Dir::ToRight, Dir::ToLeft, Dir::Delete, Dir::None];
    let at = order.iter().position(|d| *d == row.dir).unwrap_or(3);
    (1..=4)
        .map(|k| order[(at + k) % 4])
        .find(|d| can(*d))
        .unwrap_or(Dir::None)
}

/// Rows sorted by path; dirs present on both sides are descended (with `recursive`), not listed;
/// a dir on one side only is one row. Stops early when `stop` is set.
pub fn compare(left: &Path, right: &Path, o: &Options, stop: &AtomicBool) -> Vec<Row> {
    let mut out = Vec::new();
    walk(left, right, Path::new(""), o, stop, &mut out);
    out
}

fn list(dir: &Path, hidden: bool) -> BTreeMap<OsString, Info> {
    let Ok(rd) = fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    rd.flatten()
        .filter(|e| hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let m = e.metadata().ok()?; // does not follow symlinks
            let ft = m.file_type();
            (ft.is_dir() || ft.is_file() || ft.is_symlink()).then(|| {
                let info = Info {
                    size: if m.is_dir() { 0 } else { m.len() },
                    mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    dir: m.is_dir(),
                    link: ft.is_symlink(),
                };
                (e.file_name(), info)
            })
        })
        .collect()
}

fn walk(left: &Path, right: &Path, rel: &Path, o: &Options, stop: &AtomicBool, out: &mut Vec<Row>) {
    let (l, r) = (
        list(&left.join(rel), o.hidden),
        list(&right.join(rel), o.hidden),
    );
    let mut names: Vec<&OsString> = l.keys().chain(r.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let path = rel.join(name);
        let (a, b) = (l.get(name).copied(), r.get(name).copied());
        let one_sided_dir = matches!((a, b), (Some(x), None) | (None, Some(x)) if x.dir);
        let state = match (a, b) {
            (Some(a), Some(b)) if a.dir && b.dir => {
                if o.recursive {
                    walk(left, right, &path, o, stop, out);
                }
                continue;
            }
            // With a mask a dir on one side is opened: only its matching files are copied.
            _ if one_sided_dir && o.mask.is_some() => {
                if o.recursive {
                    walk(left, right, &path, o, stop, out);
                }
                continue;
            }
            _ if !one_sided_dir
                && o.mask
                    .as_ref()
                    .is_some_and(|m| !m.matches(&name.to_string_lossy())) =>
            {
                continue;
            }
            (Some(_), None) => State::LeftOnly,
            (None, Some(_)) => State::RightOnly,
            // Different kinds (file / dir / symlink): no side is "newer", never an arrow.
            (Some(a), Some(b)) if a.dir != b.dir || a.link != b.link => State::Differ,
            (Some(a), Some(b)) => files(&left.join(&path), &right.join(&path), a, b, o),
            (None, None) => continue,
        };
        let dir = if o.mirror {
            mirror_dir(state, a, b)
        } else {
            default_dir(state)
        };
        out.push(Row {
            rel: path,
            left: a,
            right: b,
            state,
            dir,
        });
    }
}

fn files(lp: &Path, rp: &Path, a: Info, b: Info, o: &Options) -> State {
    let same_size = a.size == b.size;
    let bytes = || same_bytes(lp, rp).unwrap_or(false);
    if o.ignore_date {
        return if same_size && (!o.content || bytes()) {
            State::Same
        } else {
            State::Differ
        };
    }
    let same_date = same_time(a.mtime, b.mtime);
    if same_size && same_date {
        return if !o.content || bytes() {
            State::Same
        } else {
            State::Differ
        };
    }
    if same_size && o.content && bytes() {
        return State::Same;
    }
    if same_date {
        return State::Differ;
    }
    if a.mtime > b.mtime {
        State::LeftNewer
    } else {
        State::RightNewer
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

/// Source → target copies.
pub type Pairs = Vec<(PathBuf, PathBuf)>;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub to_right: Pairs,
    pub to_left: Pairs,
    /// Paths to delete (the side that has the row).
    pub delete: Vec<PathBuf>,
}

/// What the arrows do.
pub fn plan(left: &Path, right: &Path, rows: &[Row]) -> Plan {
    let mut p = Plan::default();
    for row in rows {
        match row.dir {
            Dir::ToRight => p.to_right.push((left.join(&row.rel), right.join(&row.rel))),
            Dir::ToLeft => p.to_left.push((right.join(&row.rel), left.join(&row.rel))),
            Dir::Delete => match (row.left.is_some(), row.right.is_some()) {
                (true, false) => p.delete.push(left.join(&row.rel)),
                (false, true) => p.delete.push(right.join(&row.rel)),
                _ => {} // both or neither: deleting would be a guess
            },
            Dir::None => {}
        }
    }
    p
}

/// Shift+F2: files to mark on each side — missing opposite, newer, or same date with another size.
pub fn compare_lists(left: &[Entry], right: &[Entry]) -> (Vec<OsString>, Vec<OsString>) {
    let files = |v: &[Entry]| -> BTreeMap<OsString, (u64, SystemTime)> {
        v.iter()
            .filter(|e| !e.is_dir())
            .map(|e| (e.os_name.clone(), (e.size, e.mtime)))
            .collect()
    };
    let (l, r) = (files(left), files(right));
    let (mut ml, mut mr) = (Vec::new(), Vec::new());
    for (name, &(ls, lt)) in &l {
        match r.get(name) {
            None => ml.push(name.clone()),
            Some(&(rs, rt)) if same_time(lt, rt) => {
                if ls != rs {
                    ml.push(name.clone());
                    mr.push(name.clone());
                }
            }
            Some(&(_, rt)) if lt > rt => ml.push(name.clone()),
            Some(_) => mr.push(name.clone()),
        }
    }
    mr.extend(r.keys().filter(|n| !l.contains_key(*n)).cloned());
    mr.sort();
    (ml, mr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn opts() -> Options {
        Options {
            recursive: true,
            content: false,
            ignore_date: false,
            hidden: false,
            ..Options::default()
        }
    }

    fn put(p: &Path, s: &str, secs: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn pair() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (l, r) = (d.path().join("l"), d.path().join("r"));
        fs::create_dir_all(&l).unwrap();
        fs::create_dir_all(&r).unwrap();
        (d, l, r)
    }

    fn states(l: &Path, r: &Path, o: &Options) -> Vec<(String, State)> {
        compare(l, r, o, &AtomicBool::new(false))
            .into_iter()
            .map(|x| (x.rel.to_string_lossy().into_owned(), x.state))
            .collect()
    }

    #[test]
    fn left_and_right_only() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1000);
        put(&r.join("b"), "1", 1000);
        assert_eq!(
            states(&l, &r, &opts()),
            [
                ("a".into(), State::LeftOnly),
                ("b".into(), State::RightOnly)
            ]
        );
    }

    #[test]
    fn newer_by_mtime() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 2000);
        put(&r.join("a"), "1", 1000);
        put(&l.join("b"), "1", 1000);
        put(&r.join("b"), "22", 2000);
        assert_eq!(
            states(&l, &r, &opts()),
            [
                ("a".into(), State::LeftNewer),
                ("b".into(), State::RightNewer)
            ]
        );
    }

    #[test]
    fn two_second_tolerance() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1002);
        put(&r.join("a"), "1", 1000);
        assert_eq!(states(&l, &r, &opts()), [("a".into(), State::Same)]);
    }

    #[test]
    fn same_date_other_size_differs() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1000);
        put(&r.join("a"), "22", 1000);
        assert_eq!(states(&l, &r, &opts()), [("a".into(), State::Differ)]);
    }

    #[test]
    fn file_vs_dir_differs() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1000);
        fs::create_dir(r.join("a")).unwrap();
        let rows = compare(&l, &r, &opts(), &AtomicBool::new(false));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, State::Differ);
        assert_eq!(rows[0].dir, Dir::None);
    }

    #[test]
    fn content_mode() {
        let (_d, l, r) = pair();
        put(&l.join("same"), "abc", 2000);
        put(&r.join("same"), "abc", 1000);
        put(&l.join("diff"), "abc", 1000);
        put(&r.join("diff"), "xyz", 1000);
        let o = Options {
            content: true,
            ..opts()
        };
        assert_eq!(
            states(&l, &r, &o),
            [("diff".into(), State::Differ), ("same".into(), State::Same)]
        );
    }

    #[test]
    fn ignore_date_mode() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "abc", 5000);
        put(&r.join("a"), "abc", 1000);
        put(&l.join("b"), "abc", 5000);
        put(&r.join("b"), "abcd", 1000);
        let o = Options {
            ignore_date: true,
            ..opts()
        };
        assert_eq!(
            states(&l, &r, &o),
            [("a".into(), State::Same), ("b".into(), State::Differ)]
        );
    }

    #[test]
    fn one_sided_dir_is_one_row() {
        let (_d, l, r) = pair();
        put(&l.join("only/x/y"), "1", 1000);
        put(&l.join("both/n"), "1", 2000);
        put(&r.join("both/n"), "1", 1000);
        assert_eq!(
            states(&l, &r, &opts()),
            [
                ("both/n".into(), State::LeftNewer),
                ("only".into(), State::LeftOnly)
            ]
        );
    }

    #[test]
    fn recursive_off_lists_top_only() {
        let (_d, l, r) = pair();
        put(&l.join("both/n"), "1", 2000);
        put(&r.join("both/n"), "1", 1000);
        put(&l.join("top"), "1", 1000);
        let o = Options {
            recursive: false,
            ..opts()
        };
        assert_eq!(states(&l, &r, &o), [("top".into(), State::LeftOnly)]);
    }

    #[test]
    fn hidden_skipped() {
        let (_d, l, r) = pair();
        put(&l.join(".h"), "1", 1000);
        assert!(states(&l, &r, &opts()).is_empty());
        assert_eq!(
            states(
                &l,
                &r,
                &Options {
                    hidden: true,
                    ..opts()
                }
            )
            .len(),
            1
        );
    }

    #[test]
    fn plan_follows_arrows() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 2000);
        put(&r.join("a"), "1", 1000);
        put(&r.join("sub/b"), "1", 1000);
        put(&l.join("c"), "1", 1000);
        let mut rows = compare(&l, &r, &opts(), &AtomicBool::new(false));
        assert_eq!(
            rows.iter().map(|x| x.dir).collect::<Vec<_>>(),
            [Dir::ToRight, Dir::ToRight, Dir::ToLeft]
        );
        rows[1].dir = Dir::None; // "c" skipped by the user
        let p = plan(&l, &r, &rows);
        assert_eq!(p.to_right, [(l.join("a"), r.join("a"))]);
        assert_eq!(p.to_left, [(r.join("sub"), l.join("sub"))]);
    }

    fn entry(name: &str, size: u64, secs: u64, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: String::new(),
            size,
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            kind: if dir {
                crate::listing::Kind::Dir
            } else {
                crate::listing::Kind::File
            },
            is_link: false,
            mode: 0o644,
            owner: None,
            target: None,
        }
    }

    #[test]
    fn compare_lists_marks_unique_and_newer() {
        let left = [
            entry("..", 0, 0, true),
            entry("a", 1, 2000, false),
            entry("same", 1, 1000, false),
            entry("size", 1, 1000, false),
            entry("only", 1, 1, false),
            entry("d", 0, 9, true),
        ];
        let right = [
            entry("a", 1, 1000, false),
            entry("same", 1, 1001, false),
            entry("size", 2, 1000, false),
            entry("ronly", 1, 1, false),
        ];
        let (ml, mr) = compare_lists(&left, &right);
        assert_eq!(ml, ["a", "only", "size"].map(OsString::from));
        assert_eq!(mr, ["ronly", "size"].map(OsString::from));
    }

    #[test]
    fn regression_symlink_vs_file_has_no_arrow() {
        let (_d, l, r) = pair();
        std::os::unix::fs::symlink("nowhere", l.join("s")).unwrap();
        put(&r.join("s"), "important data here", 1000);
        let rows = compare(&l, &r, &opts(), &AtomicBool::new(false));
        assert_eq!((rows[0].state, rows[0].dir), (State::Differ, Dir::None));
    }

    #[test]
    fn regression_slack_is_two_seconds_not_three() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1000);
        put(&r.join("a"), "1", 1000);
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_002_900);
        fs::File::options()
            .write(true)
            .open(l.join("a"))
            .unwrap()
            .set_modified(t)
            .unwrap();
        assert_eq!(states(&l, &r, &opts()), [("a".into(), State::LeftNewer)]);
    }

    fn arrows(l: &Path, r: &Path, o: &Options) -> Vec<(String, Dir)> {
        compare(l, r, o, &AtomicBool::new(false))
            .into_iter()
            .map(|x| (x.rel.to_string_lossy().into_owned(), x.dir))
            .collect()
    }

    #[test]
    fn mirror_makes_the_right_a_copy_of_the_left() {
        let (_d, l, r) = pair();
        put(&l.join("a"), "1", 1000); // left only
        put(&r.join("b"), "1", 1000); // right only → delete
        put(&l.join("c"), "1", 1000);
        put(&r.join("c"), "1", 3000); // right newer: the left still wins
        put(&l.join("d"), "1", 1000);
        put(&r.join("d"), "1", 1000); // same
        fs::create_dir(l.join("e")).unwrap();
        put(&r.join("e"), "1", 1000); // dir against file: no arrow, never deleted
        let o = Options {
            mirror: true,
            ..opts()
        };
        assert_eq!(
            arrows(&l, &r, &o),
            [
                ("a".into(), Dir::ToRight),
                ("b".into(), Dir::Delete),
                ("c".into(), Dir::ToRight),
                ("d".into(), Dir::None),
                ("e".into(), Dir::None),
            ]
        );
    }

    #[test]
    fn plan_lists_deletions_on_the_existing_side() {
        let (_d, l, r) = pair();
        put(&r.join("b"), "1", 1000);
        put(&l.join("a"), "1", 1000);
        let mut rows = compare(&l, &r, &opts(), &AtomicBool::new(false));
        for row in &mut rows {
            row.dir = Dir::Delete;
        }
        let p = plan(&l, &r, &rows);
        assert!(p.to_right.is_empty() && p.to_left.is_empty());
        assert_eq!(p.delete, [l.join("a"), r.join("b")]);
    }

    #[test]
    fn mask_keeps_matching_files_and_opens_one_sided_dirs() {
        let (_d, l, r) = pair();
        put(&l.join("a.rs"), "1", 1000);
        put(&l.join("a.txt"), "1", 1000);
        put(&l.join("only/b.rs"), "1", 1000);
        put(&l.join("only/b.txt"), "1", 1000);
        let o = Options {
            mask: Some(crate::mask::Mask::parse("*.rs")),
            ..opts()
        };
        assert_eq!(
            arrows(&l, &r, &o),
            [
                ("a.rs".into(), Dir::ToRight),
                ("only/b.rs".into(), Dir::ToRight)
            ]
        );
        // Without a mask the one-sided dir stays one row.
        assert_eq!(arrows(&l, &r, &opts()).len(), 3);
    }

    #[test]
    fn click_cycles_through_what_the_row_allows() {
        let row = |left: bool, right: bool| Row {
            rel: "x".into(),
            left: left.then_some(Info {
                size: 1,
                mtime: SystemTime::UNIX_EPOCH,
                dir: false,
                link: false,
            }),
            right: right.then_some(Info {
                size: 1,
                mtime: SystemTime::UNIX_EPOCH,
                dir: false,
                link: false,
            }),
            state: State::Same,
            dir: Dir::None,
        };
        let cycle = |mut r: Row| {
            let mut seen = Vec::new();
            for _ in 0..4 {
                r.dir = next_dir(&r);
                seen.push(r.dir);
            }
            seen
        };
        assert_eq!(
            cycle(row(true, true)),
            [Dir::ToRight, Dir::ToLeft, Dir::None, Dir::ToRight]
        );
        assert_eq!(
            cycle(row(true, false)),
            [Dir::ToRight, Dir::Delete, Dir::None, Dir::ToRight]
        );
        assert_eq!(
            cycle(row(false, true)),
            [Dir::ToLeft, Dir::Delete, Dir::None, Dir::ToLeft]
        );
    }
}
