use crate::listing::Entry;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Ext,
    Size,
    Date,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Case-insensitive comparison where digit runs compare as numbers (`file2 < file10`).
/// Names equal under that rule fall back to byte order, so the result is never `Equal` for different strings.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let ord = match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (n, m) = (take_digits(&mut x), take_digits(&mut y));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                n.len().cmp(&m.len()).then_with(|| n.cmp(m))
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                c.to_lowercase().cmp(d.to_lowercase())
            }
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.next_if(char::is_ascii_digit) {
        s.push(c);
    }
    s
}

/// TC order: dirs above files in both directions; dirs sort by name when sorting by size.
pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        match (a.is_dir(), b.is_dir()) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let key = if a.is_dir() && sort.key == SortKey::Size {
            SortKey::Name
        } else {
            sort.key
        };
        let ord = match key {
            SortKey::Name => Ordering::Equal,
            SortKey::Ext => natural_cmp(&a.ext, &b.ext),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Date => a.mtime.cmp(&b.mtime),
        }
        .then_with(|| natural_cmp(&a.name, &b.name))
        // lossy names can collide; the real name keeps the order total
        .then_with(|| a.os_name.cmp(&b.os_name));
        if sort.asc { ord } else { ord.reverse() }
    });
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
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
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
        sort_entries(&mut v, Sort::default());
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
                sort_entries(&mut v, Sort { key, asc });
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
        sort_entries(&mut v, Sort::default());
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
        );
        assert_eq!(names(&v), ["adir", "zdir", "small1", "small2", "big"]);
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
        );
        assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Date,
                asc: true,
            },
        );
        assert_eq!(names(&v), ["y.md", "a.rs", "x.rs"]);
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Date,
                asc: false,
            },
        );
        assert_eq!(names(&v), ["x.rs", "a.rs", "y.md"]);
    }
}
