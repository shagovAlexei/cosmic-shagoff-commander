//! Ctrl+M multi-rename: TC-style name masks, find/replace, case, collision check and a safe rename order.

use jiff::tz::TimeZone;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;
use std::{fs, io};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Keep,
    Upper,
    Lower,
    /// First letter upper, the rest lower.
    Title,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counter {
    pub start: i64,
    pub step: i64,
    /// Zero-padded width; 1 = no padding.
    pub digits: usize,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            start: 1,
            step: 1,
            digits: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub ext: String,
    pub find: String,
    pub replace: String,
    pub case: Case,
    pub counter: Counter,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: "[N]".into(),
            ext: "[E]".into(),
            find: String::new(),
            replace: String::new(),
            case: Case::Keep,
            counter: Counter::default(),
        }
    }
}

/// Cap for the counter width: a typo like 999999 must not build a megabyte name.
pub const MAX_DIGITS: usize = 20;

/// New name for the `index`-th file (0-based): masks, then find/replace, then case.
pub fn new_name(rule: &Rule, old: &str, mtime: SystemTime, index: usize, tz: &TimeZone) -> String {
    let (name, ext) = split(old);
    let c = rule.counter;
    let n = c.start.saturating_add(
        i64::try_from(index)
            .unwrap_or(i64::MAX)
            .saturating_mul(c.step),
    );
    let cx = Ctx {
        name,
        ext,
        counter: format!("{n:0w$}", w = c.digits.min(MAX_DIGITS)),
        date: jiff::Timestamp::try_from(mtime)
            .ok()
            .map(|t| t.to_zoned(tz.clone())),
    };
    let mut out = expand(&rule.name, &cx);
    let e = expand(&rule.ext, &cx);
    if !e.is_empty() {
        out.push('.');
        out.push_str(&e);
    }
    if !rule.find.is_empty() {
        out = out.replace(&rule.find, &rule.replace);
    }
    apply_case(&out, rule.case)
}

struct Ctx<'a> {
    name: &'a str,
    ext: &'a str,
    counter: String,
    date: Option<jiff::Zoned>,
}

/// `a.tar.gz` → (`a.tar`, `gz`); `.bashrc` and `Makefile` have no extension.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Replaces every known `[...]` token; `[[` / `]]` are literal brackets; anything else stays text.
fn expand(mask: &str, cx: &Ctx) -> String {
    let mut out = String::new();
    let mut rest = mask;
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("[[") {
            out.push('[');
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("]]") {
            out.push(']');
            rest = r;
            continue;
        }
        if c == '['
            && let Some(end) = rest.find(']')
            && let Some(v) = token(&rest[1..end], cx)
        {
            out.push_str(&v);
            rest = &rest[end + 1..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn token(t: &str, cx: &Ctx) -> Option<String> {
    let date = |f: &str| {
        cx.date
            .as_ref()
            .map(|z| z.strftime(f).to_string())
            .unwrap_or_default()
    };
    Some(match t {
        "C" => cx.counter.clone(),
        "Y" => date("%Y"),
        "M" => date("%m"),
        "D" => date("%d"),
        "h" => date("%H"),
        "m" => date("%M"),
        "s" => date("%S"),
        _ => {
            let (src, spec) = match t.split_at_checked(1)? {
                ("N", s) => (cx.name, s),
                ("E", s) => (cx.ext, s),
                _ => return None,
            };
            if spec.is_empty() {
                return Some(src.to_string());
            }
            let (from, to): (usize, Option<usize>) = match spec.split_once('-') {
                Some((a, "")) => (a.parse().ok()?, None),
                Some((a, b)) => (a.parse().ok()?, Some(b.parse().ok()?)),
                None => {
                    let n = spec.parse().ok()?;
                    (n, Some(n))
                }
            };
            if from == 0 {
                return None;
            }
            let take = to.map_or(usize::MAX, |to| (to + 1).saturating_sub(from));
            src.chars().skip(from - 1).take(take).collect()
        }
    })
}

fn apply_case(s: &str, case: Case) -> String {
    match case {
        Case::Keep => s.to_string(),
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
        Case::Title => {
            let mut chars = s.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Empty, `.`, `..`, or contains `/` or NUL.
    BadName,
    /// Two or more rows get this name.
    Duplicate,
    /// A file in the dir that is not being renamed already has this name.
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub old: String,
    pub new: String,
    pub problem: Option<Problem>,
}

/// `files`: (name, mtime) in panel order; `taken`: names in the dir that are not being renamed.
pub fn preview(
    rule: &Rule,
    files: &[(String, SystemTime)],
    taken: &HashSet<String>,
    tz: &TimeZone,
) -> Vec<Row> {
    let rows = files
        .iter()
        .enumerate()
        .map(|(i, (old, mtime))| Row {
            old: old.clone(),
            new: new_name(rule, old, *mtime, i, tz),
            problem: None,
        })
        .collect();
    check(rows, taken)
}

/// Sets each row's problem. A new name equal to another renamed row's old name is fine (chain/swap).
fn check(mut rows: Vec<Row>, taken: &HashSet<String>) -> Vec<Row> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for r in &rows {
        *count.entry(r.new.clone()).or_default() += 1;
    }
    for r in &mut rows {
        let n = r.new.as_str();
        r.problem = if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\0']) {
            Some(Problem::BadName)
        } else if count[n] > 1 {
            Some(Problem::Duplicate)
        } else if taken.contains(n) {
            Some(Problem::Exists)
        } else {
            None
        };
    }
    rows
}

/// Names in `dir` (hidden ones too) other than the files being renamed.
pub fn other_names(dir: &Path, renamed: &[(String, SystemTime)]) -> io::Result<HashSet<String>> {
    let skip: HashSet<&str> = renamed.iter().map(|(n, _)| n.as_str()).collect();
    let mut out = HashSet::new();
    for e in fs::read_dir(dir)? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if !skip.contains(name.as_str()) {
            out.insert(name);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-02 14:05:09 UTC.
    fn when() -> SystemTime {
        jiff::civil::date(2026, 10, 2)
            .at(14, 5, 9, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .into()
    }

    fn rule(name: &str, ext: &str) -> Rule {
        Rule {
            name: name.into(),
            ext: ext.into(),
            ..Rule::default()
        }
    }

    fn nn(r: &Rule, old: &str) -> String {
        new_name(r, old, when(), 0, &TimeZone::UTC)
    }

    #[test]
    fn default_rule_keeps_the_name() {
        for n in ["a.txt", "Makefile", ".bashrc", "a.tar.gz", "фото.JPG"] {
            assert_eq!(nn(&Rule::default(), n), n);
        }
    }

    #[test]
    fn name_and_ext_ranges() {
        let f = "abcdef.html";
        assert_eq!(nn(&rule("[N3]", ""), f), "c");
        assert_eq!(nn(&rule("[N2-4]", ""), f), "bcd");
        assert_eq!(nn(&rule("[N4-]", ""), f), "def");
        assert_eq!(nn(&rule("[N10-20]", ""), f), ""); // past the end → empty
        assert_eq!(nn(&rule("[N]", "[E1-2]"), f), "abcdef.ht");
        assert_eq!(nn(&rule("[N]", "[E2]"), f), "abcdef.t");
        assert_eq!(nn(&rule("[N]", "[E3-]"), f), "abcdef.ml");
    }

    #[test]
    fn ranges_count_chars_not_bytes() {
        assert_eq!(nn(&rule("[N2-3]", "[E]"), "привет.txt"), "ри.txt");
    }

    #[test]
    fn extension_is_after_the_last_dot_and_not_for_dotfiles() {
        assert_eq!(nn(&rule("[N]_x", "[E]"), "a.tar.gz"), "a.tar_x.gz");
        assert_eq!(nn(&rule("[N]_x", "[E]"), ".bashrc"), ".bashrc_x");
        assert_eq!(nn(&rule("[N]_x", "[E]"), "Makefile"), "Makefile_x");
        assert_eq!(nn(&rule("[N]", "new"), "Makefile"), "Makefile.new");
    }

    #[test]
    fn literal_brackets_and_unknown_masks_stay_text() {
        assert_eq!(nn(&rule("[[[N]]]", ""), "a.txt"), "[a]");
        assert_eq!(nn(&rule("[X]-[N", ""), "a.txt"), "[X]-[N");
        assert_eq!(nn(&rule("[N0]", ""), "a.txt"), "[N0]"); // positions start at 1
        assert_eq!(nn(&rule("[Na-b]", ""), "a.txt"), "[Na-b]");
    }

    #[test]
    fn date_and_time_of_mtime_in_tz() {
        assert_eq!(
            nn(&rule("[Y]-[M]-[D]_[h][m][s]", ""), "a"),
            "2026-10-02_140509"
        );
        let plus3 = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(new_name(&rule("[h]", ""), "a", when(), 0, &plus3), "17");
    }

    #[test]
    fn counter_start_step_digits() {
        let mut r = rule("[N]_[C]", "[E]");
        r.counter = Counter {
            start: 5,
            step: 10,
            digits: 3,
        };
        let names: Vec<String> = (0..3)
            .map(|i| new_name(&r, "a.txt", when(), i, &TimeZone::UTC))
            .collect();
        assert_eq!(names, ["a_005.txt", "a_015.txt", "a_025.txt"]);
        r.counter = Counter {
            start: 2,
            step: -1,
            digits: 1,
        };
        assert_eq!(new_name(&r, "a.txt", when(), 3, &TimeZone::UTC), "a_-1.txt");
    }

    #[test]
    fn counter_is_capped_and_saturates() {
        let mut r = rule("[C]", "");
        r.counter = Counter {
            start: i64::MAX,
            step: i64::MAX,
            digits: 999_999,
        };
        let n = new_name(&r, "a", when(), 7, &TimeZone::UTC);
        assert_eq!(n.len(), MAX_DIGITS.max(i64::MAX.to_string().len()));
        assert!(n.ends_with(&i64::MAX.to_string()));
    }

    #[test]
    fn find_replace_every_occurrence_on_the_full_name() {
        let mut r = Rule::default();
        r.find = "a".into();
        r.replace = "o".into();
        assert_eq!(nn(&r, "banana.tar"), "bonono.tor");
        r.find = String::new(); // empty find → skipped
        assert_eq!(nn(&r, "banana.tar"), "banana.tar");
    }

    #[test]
    fn case_modes_apply_last_and_handle_cyrillic() {
        let mut r = Rule::default();
        r.find = "x".into();
        r.replace = "Y".into();
        for (case, want) in [
            (Case::Keep, "Фото YY.Jpg"),
            (Case::Upper, "ФОТО YY.JPG"),
            (Case::Lower, "фото yy.jpg"),
            (Case::Title, "Фото yy.jpg"),
        ] {
            r.case = case;
            assert_eq!(nn(&r, "Фото xY.Jpg"), want, "{case:?}");
        }
        r.case = Case::Title;
        assert_eq!(nn(&r, "élan"), "Élan");
    }

    fn files(names: &[&str]) -> Vec<(String, SystemTime)> {
        names.iter().map(|n| (n.to_string(), when())).collect()
    }

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn row(old: &str, new: &str) -> Row {
        Row {
            old: old.into(),
            new: new.into(),
            problem: None,
        }
    }

    fn problems(rows: &[Row]) -> Vec<Option<Problem>> {
        rows.iter().map(|r| r.problem).collect()
    }

    #[test]
    fn preview_numbers_rows_in_order() {
        let rows = preview(
            &rule("x[C]", "[E]"),
            &files(&["b.txt", "a.txt"]),
            &taken(&[]),
            &TimeZone::UTC,
        );
        let new: Vec<&str> = rows.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(new, ["x1.txt", "x2.txt"]);
        assert_eq!(rows[0].old, "b.txt");
        assert_eq!(problems(&rows), [None, None]);
    }

    #[test]
    fn bad_names_are_flagged() {
        for mask in ["", ".", "..", "a/b"] {
            let rows = preview(
                &rule(mask, ""),
                &files(&["a.txt"]),
                &taken(&[]),
                &TimeZone::UTC,
            );
            assert_eq!(problems(&rows), [Some(Problem::BadName)], "{mask:?}");
        }
    }

    #[test]
    fn duplicates_flag_every_row() {
        let rows = preview(
            &rule("same", "[E]"),
            &files(&["a.txt", "b.txt", "c.md"]),
            &taken(&[]),
            &TimeZone::UTC,
        );
        assert_eq!(
            problems(&rows),
            [Some(Problem::Duplicate), Some(Problem::Duplicate), None]
        );
    }

    #[test]
    fn unchanged_row_collides_as_duplicate() {
        // a.txt → b.txt while b.txt keeps its name: two rows want b.txt.
        let mut r = Rule::default();
        r.find = "a".into();
        r.replace = "b".into();
        let rows = preview(&r, &files(&["a.txt", "b.txt"]), &taken(&[]), &TimeZone::UTC);
        assert_eq!(
            problems(&rows),
            [Some(Problem::Duplicate), Some(Problem::Duplicate)]
        );
    }

    #[test]
    fn name_of_a_file_not_renamed_is_exists() {
        let rows = preview(
            &rule("c", "[E]"),
            &files(&["a.txt"]),
            &taken(&["c.txt"]),
            &TimeZone::UTC,
        );
        assert_eq!(problems(&rows), [Some(Problem::Exists)]);
    }

    #[test]
    fn chain_and_swap_are_not_problems() {
        // A new name that is another renamed row's old name is free: `taken` never holds it.
        let chain = vec![row("a", "b"), row("b", "c")];
        assert_eq!(check(chain.clone(), &taken(&[])), chain);
        let swap = vec![row("a", "b"), row("b", "a")];
        assert_eq!(check(swap.clone(), &taken(&[])), swap);
    }

    #[test]
    fn other_names_includes_hidden_and_skips_renamed() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a", "b", ".hidden"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        let got = other_names(d.path(), &files(&["a"])).unwrap();
        assert_eq!(got, taken(&["b", ".hidden"]));
        assert!(other_names(&d.path().join("missing"), &[]).is_err());
    }
}
