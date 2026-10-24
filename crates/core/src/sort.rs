use crate::listing::Entry;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Ext,
    Size,
    Date,
    /// The Owner / Group column headers: by name, the number where unknown.
    Owner,
    Group,
    /// Ctrl+F7 (TC `cm_SrcUnsorted`): as the dir lists them; dirs still first.
    Unsorted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sort {
    pub key: SortKey,
    pub asc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            asc: true,
        }
    }
}

fn owner_name(e: &Entry, user: bool) -> String {
    let o = crate::owners::shared();
    match (e.owner, user) {
        (Some((u, _)), true) => o.user(u),
        (Some((_, g)), false) => o.group(g),
        (None, _) => String::new(),
    }
}

/// Case-insensitive comparison where digit runs compare as numbers (`file2 < file10`).
/// Names equal under that rule fall back to byte order, so the result is never `Equal` for different strings.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    // Byte offsets, always on char boundaries: ASCII steps one byte, other chars their length.
    // No allocation per comparison: a 400 000-entry branch view sorts millions of times.
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        let (c, d) = match (x.get(i), y.get(j)) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&c), Some(&d)) => (c, d),
        };
        let ord = if c.is_ascii_digit() && d.is_ascii_digit() {
            let (n, m) = (digits(x, &mut i), digits(y, &mut j));
            n.len().cmp(&m.len()).then_with(|| n.cmp(m))
        } else if c.is_ascii() && d.is_ascii() {
            i += 1;
            j += 1;
            c.to_ascii_lowercase().cmp(&d.to_ascii_lowercase())
        } else {
            let (Some(c), Some(d)) = (a[i..].chars().next(), b[j..].chars().next()) else {
                return a.cmp(b); // unreachable: both have bytes left on a boundary
            };
            i += c.len_utf8();
            j += d.len_utf8();
            c.to_lowercase().cmp(d.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

/// The digit run at `*k` without leading zeros; `*k` moves past it.
fn digits<'a>(s: &'a [u8], k: &mut usize) -> &'a [u8] {
    let start = *k;
    while s.get(*k).is_some_and(u8::is_ascii_digit) {
        *k += 1;
    }
    let run = &s[start..*k];
    &run[run.iter().take_while(|&&b| b == b'0').count()..]
}

/// TC `cm_SwitchDirSort`: dirs always by name, ascending, whatever the files go by. Program-wide,
/// like the setting (`set_dirs_by_name` from the config).
static DIRS_BY_NAME: AtomicBool = AtomicBool::new(false);

pub fn set_dirs_by_name(on: bool) {
    DIRS_BY_NAME.store(on, AtomicOrdering::Relaxed);
}

/// TC order: dirs above files in both directions. By size, dirs go by their counted size
/// (`dir_sizes`, Space / Alt+Shift+Enter; uncounted as 0), so by name until counted.
pub fn sort_entries(entries: &mut [Entry], sort: Sort, dir_sizes: &HashMap<OsString, u64>) {
    let by_name = DIRS_BY_NAME.load(AtomicOrdering::Relaxed);
    entries.sort_by(|a, b| entry_cmp(a, b, sort, dir_sizes, by_name));
}

/// Already in `sort` order: a listing sorted in the background needs no second sort on the UI
/// thread (a 400 000-file branch takes a second to sort, one pass to check).
pub fn is_sorted(entries: &[Entry], sort: Sort, dir_sizes: &HashMap<OsString, u64>) -> bool {
    let by_name = DIRS_BY_NAME.load(AtomicOrdering::Relaxed);
    entries.is_sorted_by(|a, b| entry_cmp(a, b, sort, dir_sizes, by_name) != Ordering::Greater)
}

fn entry_cmp(
    a: &Entry,
    b: &Entry,
    mut sort: Sort,
    dir_sizes: &HashMap<OsString, u64>,
    dirs_by_name: bool,
) -> Ordering {
    let size = |e: &Entry| match e.is_dir() {
        true => dir_sizes.get(&e.os_name).copied().unwrap_or(0),
        false => e.size,
    };
    match (a.is_dir(), b.is_dir()) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) if dirs_by_name => sort = Sort::default(),
        _ => {}
    }
    let ord = match sort.key {
        // Stable sort: the order they came in.
        SortKey::Unsorted => return Ordering::Equal,
        SortKey::Name => Ordering::Equal,
        SortKey::Ext => natural_cmp(&a.ext, &b.ext),
        SortKey::Size => size(a).cmp(&size(b)),
        SortKey::Date => a.mtime.cmp(&b.mtime),
        SortKey::Owner => owner_name(a, true).cmp(&owner_name(b, true)),
        SortKey::Group => owner_name(a, false).cmp(&owner_name(b, false)),
    }
    .then_with(|| natural_cmp(&a.name, &b.name))
    // lossy names can collide; the real name keeps the order total
    .then_with(|| a.os_name.cmp(&b.os_name));
    if sort.asc { ord } else { ord.reverse() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::Kind;
    use std::time::{Duration, UNIX_EPOCH};

    fn e(name: &str, kind: Kind, size: u64, secs: u64) -> Entry {
        let ext = match (kind, name.rfind('.')) {
            (Kind::File, Some(i)) if i > 0 => name[i + 1..].to_string(),
            _ => String::new(),
        };
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext,
            size,
            mtime: UNIX_EPOCH + Duration::from_secs(secs),
            kind,
            is_link: false,
            mode: 0,
            owner: None,
            target: None,
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn sorts_by_owner_and_group_names() {
        let mut v = vec![
            e("a", Kind::File, 0, 0),
            e("b", Kind::File, 0, 0),
            e("c", Kind::File, 0, 0),
        ];
        v[0].owner = Some((0, 0));
        v[1].owner = None;
        v[2].owner = Some((0, 0));
        let s = Sort {
            key: SortKey::Owner,
            asc: false,
        };
        sort_entries(&mut v, s, &HashMap::new());
        // `root` before nobody-known: unowned (empty name) last when descending.
        assert_eq!(names(&v), ["c", "a", "b"]);
        let g = Sort {
            key: SortKey::Group,
            asc: true,
        };
        sort_entries(&mut v, g, &HashMap::new());
        assert_eq!(names(&v), ["b", "a", "c"]);
    }

    #[test]
    fn regression_presorted_listing_is_not_sorted_again_on_the_ui_thread() {
        let mut v = vec![
            e("b10.txt", Kind::File, 1, 0),
            e("B2.txt", Kind::File, 1, 0),
            e("dir", Kind::Dir, 0, 0),
            e("Ёлка", Kind::File, 1, 0),
        ];
        let (s, sizes) = (Sort::default(), HashMap::new());
        assert!(!is_sorted(&v, s, &sizes));
        sort_entries(&mut v, s, &sizes);
        assert!(is_sorted(&v, s, &sizes));
        let desc = Sort { asc: false, ..s };
        assert!(!is_sorted(&v, desc, &sizes)); // another order: the panel sorts again
    }

    #[test]
    fn natural_cmp_numbers_and_case() {
        use Ordering::*;
        assert_eq!(natural_cmp("file2", "file10"), Less);
        assert_eq!(natural_cmp("File10", "file2"), Greater);
        assert_eq!(natural_cmp("abc", "ABD"), Less);
        assert_eq!(natural_cmp("a", "ab"), Less);
        assert_eq!(natural_cmp("", "a"), Less);
        assert_eq!(natural_cmp("x", "x"), Equal);
        assert_eq!(natural_cmp("v1.9", "v1.10"), Less);
    }

    #[test]
    fn natural_cmp_tiebreak_is_deterministic() {
        use Ordering::*;
        // equal ignoring case / leading zeros → fall back to plain byte order, never Equal
        assert_eq!(natural_cmp("A", "a"), Less);
        assert_eq!(natural_cmp("a", "A"), Greater);
        assert_eq!(natural_cmp("file01", "file1"), Less);
        assert_eq!(natural_cmp("file1", "file01"), Greater);
    }

    #[test]
    fn lossy_equal_names_order_by_real_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let mk = |raw: &[u8]| {
            let mut x = e("", Kind::File, 0, 0);
            x.os_name = OsStr::from_bytes(raw).to_owned();
            x.name = x.os_name.to_string_lossy().into_owned();
            x
        };
        let mut v = vec![mk(b"a\xff"), mk(b"a\xfe")];
        sort_entries(&mut v, Sort::default(), &HashMap::new());
        assert_eq!(v[0].os_name, OsStr::from_bytes(b"a\xfe"));
    }

    #[test]
    fn dirs_on_top_for_every_key_and_direction() {
        for key in [SortKey::Name, SortKey::Ext, SortKey::Size, SortKey::Date] {
            for asc in [true, false] {
                let mut v = vec![
                    e("z.txt", Kind::File, 1, 1),
                    e("bdir", Kind::Dir, 0, 9),
                    e("a.rs", Kind::File, 900, 5),
                    e("adir", Kind::Dir, 0, 2),
                ];
                sort_entries(&mut v, Sort { key, asc }, &HashMap::new());
                let kinds: Vec<_> = v.iter().map(|e| e.kind).collect();
                assert_eq!(
                    kinds,
                    [Kind::Dir, Kind::Dir, Kind::File, Kind::File],
                    "{key:?} asc={asc}: {:?}",
                    names(&v)
                );
            }
        }
    }

    #[test]
    fn dirs_first_then_name() {
        let mut v = vec![
            e("b.txt", Kind::File, 1, 0),
            e("Zdir", Kind::Dir, 0, 0),
            e("a10", Kind::File, 1, 0),
            e("adir", Kind::Dir, 0, 0),
            e("a2", Kind::File, 1, 0),
        ];
        sort_entries(&mut v, Sort::default(), &HashMap::new());
        assert_eq!(names(&v), ["adir", "Zdir", "a2", "a10", "b.txt"]);
    }

    #[test]
    fn descending_keeps_dirs_on_top() {
        let mut v = vec![
            e("a", Kind::File, 1, 0),
            e("d1", Kind::Dir, 0, 0),
            e("b", Kind::File, 1, 0),
            e("d2", Kind::Dir, 0, 0),
        ];
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Name,
                asc: false,
            },
            &HashMap::new(),
        );
        assert_eq!(names(&v), ["d2", "d1", "b", "a"]);
    }

    #[test]
    fn by_size_dirs_by_name_files_by_size_then_name() {
        let mut v = vec![
            e("big", Kind::File, 100, 0),
            e("zdir", Kind::Dir, 0, 0),
            e("small2", Kind::File, 1, 0),
            e("adir", Kind::Dir, 0, 0),
            e("small1", Kind::File, 1, 0),
        ];
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Size,
                asc: true,
            },
            &HashMap::new(),
        );
        assert_eq!(names(&v), ["adir", "zdir", "small1", "small2", "big"]);
    }

    #[test]
    fn by_size_counted_dirs_go_by_their_size() {
        let mut v = vec![
            e("adir", Kind::Dir, 0, 0),
            e("big", Kind::Dir, 0, 0),
            e("f", Kind::File, 1, 0),
            e("mid", Kind::Dir, 0, 0),
        ];
        let sizes = HashMap::from([("big".into(), 900), ("mid".into(), 50)]);
        let by = |asc| Sort {
            key: SortKey::Size,
            asc,
        };
        sort_entries(&mut v, by(true), &sizes);
        assert_eq!(names(&v), ["adir", "mid", "big", "f"]); // uncounted as 0
        sort_entries(&mut v, by(false), &sizes);
        assert_eq!(names(&v), ["big", "mid", "adir", "f"]);
    }

    #[test]
    fn by_ext_and_by_date() {
        let mut v = vec![
            e("x.rs", Kind::File, 0, 30),
            e("y.md", Kind::File, 0, 10),
            e("a.rs", Kind::File, 0, 20),
        ];
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Ext,
                asc: true,
            },
            &HashMap::new(),
        );
        assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Date,
                asc: true,
            },
            &HashMap::new(),
        );
        assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Date,
                asc: false,
            },
            &HashMap::new(),
        );
        assert_eq!(names(&v), ["x.rs", "a.rs", "y.md"]);
    }

    #[test]
    fn unsorted_keeps_the_dir_order_and_dirs_may_go_by_name() {
        let mut v = vec![
            e("b", Kind::File, 1, 1),
            e("z", Kind::Dir, 0, 1),
            e("a", Kind::File, 9, 1),
            e("c", Kind::Dir, 0, 2),
        ];
        let un = Sort {
            key: SortKey::Unsorted,
            asc: true,
        };
        v.sort_by(|a, b| entry_cmp(a, b, un, &HashMap::new(), false));
        assert_eq!(names(&v), ["z", "c", "b", "a"]);
        let size_desc = Sort {
            key: SortKey::Size,
            asc: false,
        };
        v.sort_by(|a, b| entry_cmp(a, b, size_desc, &HashMap::new(), true));
        assert_eq!(names(&v), ["c", "z", "a", "b"]);
    }
}
