# Multi-rename (Ctrl+M) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ctrl+M opens a TC-style multi-rename dialog (name/ext masks, counter, date, find/replace, case, live preview with conflicts) and runs the renames as a normal job.

**Architecture:** All rename logic is a pure module `crates/core/src/multirename.rs` (`new_name`, `preview`, `other_names`, `plan`). The app adds `Action::MultiRename`, a boxed `Dialog::MultiRename` holding a snapshot of the files plus the form, two messages for editing, and submits through the existing `start_job(.., Job::Transfer { method: Move, pairs }, focus)`.

**Tech Stack:** Rust 2024, libcosmic (rev ef490df), `jiff` (already in core) for dates, `tempfile` for tests.

**Spec:** `.claude/docs/specs/10-multi-rename.md`

## Global Constraints

- `crates/core` must not depend on libcosmic; logic lives there with unit tests; `update()` arms stay thin.
- Positions in masks count `char`s, not bytes.
- Every new UI string goes in `fl!` with both `crates/app/i18n/en/shagoff-commander.ftl` and `crates/app/i18n/ru/shagoff-commander.ftl`.
- Tests never touch real config, trash or user data (use `tempfile`, `App::build` via `app_with`).
- `just verify` (fmt --check + clippy -D warnings + test) must pass.
- Use `/usr/bin/grep` (plain `grep` is aliased to ugrep).
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A target name that is not valid UTF-8 — it must be left out of the dialog (renaming it by its lossy name would hit the wrong path or nothing); test `mr_skips_non_utf8_names` in Task 4.
2. A hidden file in the dir while hidden files are not shown — a new name equal to it must be flagged `Exists`, not silently replaced; `taken` is read from disk (`other_names`), test `other_names_includes_hidden_and_skips_renamed` in Task 2.
3. Absurd counter input (digits `999999`, start near `i64::MAX`) — must not hang or panic; test `counter_is_capped_and_saturates` in Task 1.
4. A mask that renames one file onto another file's unchanged name (a→b while b stays b) — must be `Duplicate`, not a chain; test `unchanged_row_collides_as_duplicate` in Task 2.
5. Cancel in the middle of a swap leaves a file under its hidden temp name `.<name>.<pid>.<n>.shagoff-mr` — accepted (rename is atomic, nothing lost); noted in TESTING.md manual row in Task 5, no automated test.

---

### Task 1: core `new_name` — masks, counter, date, find/replace, case

**Files:**
- Create: `crates/core/src/multirename.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod multirename;` in alphabetical order, after `mask`)

**Interfaces:**
- Produces:
  ```rust
  pub enum Case { Keep, Upper, Lower, Title }            // Clone, Copy, Debug, Default(Keep), PartialEq, Eq
  pub struct Counter { pub start: i64, pub step: i64, pub digits: usize } // Default 1/1/1
  pub struct Rule { pub name: String, pub ext: String, pub find: String, pub replace: String,
                    pub case: Case, pub counter: Counter } // Default name "[N]", ext "[E]"
  pub fn new_name(rule: &Rule, old: &str, mtime: SystemTime, index: usize, tz: &TimeZone) -> String;
  ```

- [ ] **Step 1: Write the failing tests**

Create `crates/core/src/multirename.rs` with only the types, a `todo!()` body for `new_name`, and the tests:

```rust
//! Ctrl+M multi-rename: TC-style name masks, find/replace, case, collision check and a safe rename order.

use jiff::tz::TimeZone;
use std::time::SystemTime;

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
        Self { start: 1, step: 1, digits: 1 }
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

/// New name for the `index`-th file (0-based): masks, then find/replace, then case.
pub fn new_name(rule: &Rule, old: &str, mtime: SystemTime, index: usize, tz: &TimeZone) -> String {
    todo!()
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
        Rule { name: name.into(), ext: ext.into(), ..Rule::default() }
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
        assert_eq!(nn(&rule("[Y]-[M]-[D]_[h][m][s]", ""), "a"), "2026-10-02_140509");
        let plus3 = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(new_name(&rule("[h]", ""), "a", when(), 0, &plus3), "17");
    }

    #[test]
    fn counter_start_step_digits() {
        let mut r = rule("[N]_[C]", "[E]");
        r.counter = Counter { start: 5, step: 10, digits: 3 };
        let names: Vec<String> = (0..3)
            .map(|i| new_name(&r, "a.txt", when(), i, &TimeZone::UTC))
            .collect();
        assert_eq!(names, ["a_005.txt", "a_015.txt", "a_025.txt"]);
        r.counter = Counter { start: 2, step: -1, digits: 1 };
        assert_eq!(new_name(&r, "a.txt", when(), 3, &TimeZone::UTC), "a_-1.txt");
    }

    #[test]
    fn counter_is_capped_and_saturates() {
        let mut r = rule("[C]", "");
        r.counter = Counter { start: i64::MAX, step: i64::MAX, digits: 999_999 };
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
            (Case::Keep, "Фото YY.Jpg".into()),
            (Case::Upper, "ФОТО YY.JPG".into()),
            (Case::Lower, "фото yy.jpg".into()),
            (Case::Title, "Фото yy.jpg".into()),
        ] {
            r.case = case;
            assert_eq!(nn(&r, "Фото xY.Jpg"), want, "{case:?}");
        }
        r.case = Case::Title;
        assert_eq!(nn(&r, "élan"), "Élan");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: compile error `cannot find value MAX_DIGITS` (and, once that exists, panics at `todo!()`). Either is the expected RED.

- [ ] **Step 3: Implement**

Replace the `new_name` stub with:

```rust
/// Cap for the counter width: a typo like 999999 must not build a megabyte name.
pub const MAX_DIGITS: usize = 20;

/// New name for the `index`-th file (0-based): masks, then find/replace, then case.
pub fn new_name(rule: &Rule, old: &str, mtime: SystemTime, index: usize, tz: &TimeZone) -> String {
    let (name, ext) = split(old);
    let c = rule.counter;
    let n = c
        .start
        .saturating_add(i64::try_from(index).unwrap_or(i64::MAX).saturating_mul(c.step));
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
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        }
    }
}
```

Add `pub mod multirename;` to `crates/core/src/lib.rs` after `pub mod mask;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: all 10 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/multirename.rs crates/core/src/lib.rs
git commit -m "feat(core): multi-rename masks, counter, date, find/replace, case

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: core `preview` and `other_names` — collisions

**Files:**
- Modify: `crates/core/src/multirename.rs`

**Interfaces:**
- Consumes: `Rule`, `new_name` (Task 1).
- Produces:
  ```rust
  pub enum Problem { BadName, Duplicate, Exists }        // Clone, Copy, Debug, PartialEq, Eq
  pub struct Row { pub old: String, pub new: String, pub problem: Option<Problem> } // Clone, Debug, PartialEq, Eq
  pub fn preview(rule: &Rule, files: &[(String, SystemTime)], taken: &HashSet<String>, tz: &TimeZone) -> Vec<Row>;
  pub fn other_names(dir: &Path, renamed: &[(String, SystemTime)]) -> io::Result<HashSet<String>>;
  ```

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
    fn files(names: &[&str]) -> Vec<(String, SystemTime)> {
        names.iter().map(|n| (n.to_string(), when())).collect()
    }

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn row(old: &str, new: &str) -> Row {
        Row { old: old.into(), new: new.into(), problem: None }
    }

    fn problems(rows: &[Row]) -> Vec<Option<Problem>> {
        rows.iter().map(|r| r.problem).collect()
    }

    #[test]
    fn preview_numbers_rows_in_order() {
        let rows = preview(&rule("x[C]", "[E]"), &files(&["b.txt", "a.txt"]), &taken(&[]), &TimeZone::UTC);
        let new: Vec<&str> = rows.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(new, ["x1.txt", "x2.txt"]);
        assert_eq!(rows[0].old, "b.txt");
        assert_eq!(problems(&rows), [None, None]);
    }

    #[test]
    fn bad_names_are_flagged() {
        for mask in ["", ".", "..", "a/b"] {
            let rows = preview(&rule(mask, ""), &files(&["a.txt"]), &taken(&[]), &TimeZone::UTC);
            assert_eq!(problems(&rows), [Some(Problem::BadName)], "{mask:?}");
        }
    }

    #[test]
    fn duplicates_flag_every_row() {
        let rows = preview(&rule("same", "[E]"), &files(&["a.txt", "b.txt", "c.md"]), &taken(&[]), &TimeZone::UTC);
        assert_eq!(problems(&rows), [Some(Problem::Duplicate), Some(Problem::Duplicate), None]);
    }

    #[test]
    fn unchanged_row_collides_as_duplicate() {
        // a.txt → b.txt while b.txt keeps its name: two rows want b.txt.
        let mut r = Rule::default();
        r.find = "a".into();
        r.replace = "b".into();
        let rows = preview(&r, &files(&["a.txt", "b.txt"]), &taken(&[]), &TimeZone::UTC);
        assert_eq!(problems(&rows), [Some(Problem::Duplicate), Some(Problem::Duplicate)]);
    }

    #[test]
    fn name_of_a_file_not_renamed_is_exists() {
        let rows = preview(&rule("c", "[E]"), &files(&["a.txt"]), &taken(&["c.txt"]), &TimeZone::UTC);
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
```

Note for the implementer: `check(rows, taken) -> Vec<Row>` (private) is the collision pass split out of `preview` so a test can feed hand-made rows (a chain or swap is hard to produce from one mask).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: compile errors for `preview`, `check`, `other_names`, `Row`, `Problem`.

- [ ] **Step 3: Implement** (add after `new_name`; extend the `use` lines to `use std::collections::{HashMap, HashSet}; use std::path::Path; use std::{fs, io};`)

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: all 17 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/multirename.rs
git commit -m "feat(core): multi-rename preview with collision check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: core `plan` — safe order, chains and swaps

**Files:**
- Modify: `crates/core/src/multirename.rs`

**Interfaces:**
- Consumes: `Row` (Task 2).
- Produces: `pub fn plan(dir: &Path, rows: &[Row]) -> Vec<(PathBuf, PathBuf)>;` — pairs to run in order with `fs::rename` semantics (the app feeds them to `Job::Transfer { method: Method::Move, .. }`).

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
    /// Runs the plan with plain renames, asserting no step overwrites anything.
    fn run(dir: &Path, pairs: &[(std::path::PathBuf, std::path::PathBuf)]) {
        for (a, b) in pairs {
            assert!(!b.exists(), "{b:?} would be overwritten");
            std::fs::rename(a, b).unwrap();
        }
    }

    fn contents(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn plan_drops_unchanged_rows() {
        let d = Path::new("/d");
        assert_eq!(plan(d, &[row("a", "a"), row("b", "c")]), [(d.join("b"), d.join("c"))]);
        assert!(plan(d, &[row("a", "a")]).is_empty());
    }

    #[test]
    fn plan_orders_a_chain() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a", "b"] {
            std::fs::write(d.path().join(n), n).unwrap();
        }
        let pairs = plan(d.path(), &[row("a", "b"), row("b", "c")]);
        assert_eq!(pairs.len(), 2);
        run(d.path(), &pairs);
        assert_eq!((contents(d.path(), "b"), contents(d.path(), "c")), ("a".into(), "b".into()));
    }

    #[test]
    fn plan_swaps_through_a_temp_name() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(d.path().join(n), n).unwrap();
        }
        // a → b → c → a: a three-way cycle.
        let pairs = plan(d.path(), &[row("a", "b"), row("b", "c"), row("c", "a")]);
        assert_eq!(pairs.len(), 4);
        run(d.path(), &pairs);
        let got: Vec<String> = ["a", "b", "c"].iter().map(|n| contents(d.path(), n)).collect();
        assert_eq!(got, ["c", "a", "b"]);
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 3); // no temp left behind
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: compile error `cannot find function plan`.

- [ ] **Step 3: Implement** (add `PathBuf` to the `std::path` import)

```rust
/// Rename pairs in a safe order; unchanged rows dropped. A pair whose target is still another
/// pending file's name waits; when all remaining pairs wait (a cycle), one file is parked under a
/// hidden temp name first. Rows must be problem-free (see `preview`).
pub fn plan(dir: &Path, rows: &[Row]) -> Vec<(PathBuf, PathBuf)> {
    let mut todo: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.new != r.old)
        .map(|r| (r.old.clone(), r.new.clone()))
        .collect();
    let mut out = Vec::new();
    let mut parked = 0;
    // ponytail: O(n²) over the remaining pairs; fine for hundreds of files, index by name if it shows.
    while !todo.is_empty() {
        let busy: HashSet<&str> = todo.iter().map(|(old, _)| old.as_str()).collect();
        let free = todo.iter().position(|(_, new)| !busy.contains(new.as_str()));
        match free {
            Some(i) => {
                let (old, new) = todo.remove(i);
                out.push((dir.join(old), dir.join(new)));
            }
            None => {
                let tmp = format!(".{}.{}.{parked}.shagoff-mr", todo[0].0, std::process::id());
                parked += 1;
                out.push((dir.join(&todo[0].0), dir.join(&tmp)));
                todo[0].0 = tmp;
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shagoff-core multirename 2>&1 | tail -20`
Expected: all 20 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/multirename.rs
git commit -m "feat(core): multi-rename plan orders chains and breaks cycles

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: app — Ctrl+M, dialog, preview, submit

**Files:**
- Modify: `crates/app/src/keymap.rs` (Action enum ~line 55, letter table ~line 126, tests ~line 191)
- Modify: `crates/app/src/dialogs.rs` (Dialog enum line 20, new struct, `view` match)
- Modify: `crates/app/src/app.rs` (`Message` ~line 171, `handle` Key arm ~line 341, `DialogInput` area ~line 474, `act` focus line 697, `dialog_for` line 863, `submit_dialog` line 950, tests at the end)
- Modify: `crates/app/i18n/en/shagoff-commander.ftl`, `crates/app/i18n/ru/shagoff-commander.ftl`

**Interfaces:**
- Consumes: `multirename::{Case, Counter, Rule, Row, Problem, preview, other_names, plan}` (Tasks 1–3).
- Produces:
  ```rust
  // keymap.rs
  Action::MultiRename                         // Ctrl + physical KeyM
  // dialogs.rs
  Dialog::MultiRename(Box<MultiRename>)
  pub struct MultiRename { pub side: usize, pub dir: PathBuf, pub files: Vec<(String, SystemTime)>,
      pub taken: HashSet<String>, pub current: Option<String>, pub rule: Rule,
      pub start: String, pub step: String, pub digits: String }
  impl MultiRename { pub fn rule(&self) -> Rule; pub fn rows(&self, tz: &TimeZone) -> Vec<Row>; }
  pub enum MrField { Name, Ext, Find, Replace, Start, Step, Digits }   // Clone, Copy, Debug
  // app.rs
  Message::MrInput(MrField, String), Message::MrCase(Case)
  ```

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/keymap.rs` tests, next to `ctrl_c_x_v_clipboard`:

```rust
    #[test]
    fn ctrl_m_multi_rename() {
        assert_eq!(chr("m", Code::KeyM, CTRL), Some(Action::MultiRename));
        assert_eq!(chr("ь", Code::KeyM, CTRL), Some(Action::MultiRename)); // Russian layout
        assert_eq!(chr("m", Code::KeyM, NONE), None);
    }
```

At the end of the `tests` module in `crates/app/src/app.rs`:

```rust
    /// tmp/{a.txt, b.txt, c.txt}; pane 0 shows it with a.txt and b.txt marked (cursor on c.txt).
    fn mr_setup() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        for n in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.path().join(n), n).unwrap();
        }
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        for a in [Action::Down, Action::MarkDown, Action::MarkDown] {
            let _ = app.update(Message::Key(a));
        }
        let _ = app.update(Message::Key(Action::MultiRename));
        (tmp, app)
    }

    fn mr(app: &App) -> &dialogs::MultiRename {
        match &app.dialog {
            Some(Dialog::MultiRename(m)) => m,
            other => panic!("no multi-rename dialog: {:?}", other.is_some()),
        }
    }

    #[test]
    fn ctrl_m_opens_dialog_with_marked_files() {
        let (_tmp, app) = mr_setup();
        let m = mr(&app);
        let names: Vec<&str> = m.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
        assert!(m.taken.contains("c.txt") && !m.taken.contains("a.txt"));
        assert_eq!(m.current, None); // cursor is on c.txt, which is not renamed
    }

    #[test]
    fn mr_input_changes_preview() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
        let _ = app.update(Message::MrInput(dialogs::MrField::Digits, "2".into()));
        let rows = mr(&app).rows(&TimeZone::UTC);
        let new: Vec<&str> = rows.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(new, ["x01.txt", "x02.txt"]);
        let _ = app.update(Message::MrCase(Case::Upper));
        assert_eq!(mr(&app).rows(&TimeZone::UTC)[0].new, "X01.TXT");
    }

    #[test]
    fn mr_submit_with_problem_keeps_dialog() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "c".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(matches!(app.dialog, Some(Dialog::MultiRename(_))));
        assert!(app.job.is_none());
    }

    #[test]
    fn mr_submit_starts_move() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    }

    #[test]
    fn mr_unchanged_names_just_close() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none() && app.job.is_none());
    }

    #[test]
    fn mr_nothing_to_rename_opens_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path()); // only ".."
        let _ = app.update(Message::Key(Action::MultiRename));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn mr_skips_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(std::ffi::OsStr::from_bytes(b"bad\xff")), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // cursor on the bad name
        let _ = app.update(Message::Key(Action::MultiRename));
        assert!(app.dialog.is_none());
    }
```

Add to the test module's imports whatever is missing: `use shagoff_core::multirename::Case;` and `use shagoff_core::format::TimeZone;` (check the existing `use` block first; `OpKind`, `Dialog`, `Config`, `State` are already used by other tests).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shagoff-commander 2>&1 | tail -20`
Expected: compile errors for `Action::MultiRename`, `Dialog::MultiRename`, `Message::MrInput`, `dialogs::MrField`.

- [ ] **Step 3: keymap**

In `enum Action` after `ClipPaste,`:
```rust
    MultiRename,
```
In the letter table after the `KeyV` line:
```rust
        (Physical::Code(Code::KeyM), true, false) => Some(Action::MultiRename),
```

- [ ] **Step 4: dialogs.rs — type, field enum, view**

Imports: add `use shagoff_core::multirename::{self, Case, Problem, Row, Rule, Counter};`, `use std::collections::HashSet;`, `use std::time::SystemTime;` (merge with existing `use` lines).

Add the variant at the end of `enum Dialog`:
```rust
    /// Ctrl+M: files snapshot at open (panel order) and the form. Boxed: the form is large.
    MultiRename(Box<MultiRename>),
```

After the enum:
```rust
pub struct MultiRename {
    pub side: usize,
    pub dir: PathBuf,
    pub files: Vec<(String, SystemTime)>,
    /// Names in `dir` that are not being renamed.
    pub taken: HashSet<String>,
    /// The entry under the cursor at open, if it is being renamed: the cursor follows it.
    pub current: Option<String>,
    /// Masks, find/replace and case; the counter comes from the text fields below.
    pub rule: Rule,
    pub start: String,
    pub step: String,
    pub digits: String,
}

impl MultiRename {
    /// Counter fields that don't parse fall back to the defaults.
    pub fn rule(&self) -> Rule {
        let d = Counter::default();
        Rule {
            counter: Counter {
                start: self.start.trim().parse().unwrap_or(d.start),
                step: self.step.trim().parse().unwrap_or(d.step),
                digits: self.digits.trim().parse().unwrap_or(d.digits),
            },
            ..self.rule.clone()
        }
    }

    pub fn rows(&self, tz: &TimeZone) -> Vec<Row> {
        multirename::preview(&self.rule(), &self.files, &self.taken, tz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MrField {
    Name,
    Ext,
    Find,
    Replace,
    Start,
    Step,
    Digits,
}
```

In `view`, add an arm before `Dialog::Error`:
```rust
        Dialog::MultiRename(m) => {
            let edit = |label: String, value: &'a str, f: MrField| {
                column![
                    widget::text::caption(label),
                    widget::text_input("", value)
                        .on_input(move |s| Message::MrInput(f, s))
                        .on_submit(|_| Message::DialogSubmit),
                ]
                .spacing(2)
            };
            let name = column![
                widget::text::caption(fl!("mr-name")),
                widget::text_input("", &m.rule.name)
                    .id(input_id.clone())
                    .on_input(|s| Message::MrInput(MrField::Name, s))
                    .on_submit(|_| Message::DialogSubmit),
            ]
            .spacing(2);
            let case = |label: String, c: Case| {
                let b = if m.rule.case == c {
                    widget::button::suggested(label)
                } else {
                    widget::button::standard(label)
                };
                b.on_press(Message::MrCase(c))
            };
            let mut table = column![row![
                widget::text::heading(fl!("mr-old")).width(Length::FillPortion(1)),
                widget::text::heading(fl!("mr-new")).width(Length::FillPortion(1)),
            ]]
            .spacing(2);
            let rows = m.rows(tz);
            for r in &rows {
                let new = match r.problem {
                    None => r.new.clone(),
                    Some(p) => format!("⚠ {}  ({})", r.new, problem(p)),
                };
                table = table.push(row![
                    widget::text(r.old.clone()).width(Length::FillPortion(1)),
                    widget::text(new).width(Length::FillPortion(1)),
                ]);
            }
            let ok = rows.iter().all(|r| r.problem.is_none());
            widget::dialog()
                .title(fl!("multi-rename"))
                .control(
                    column![
                        row![name, edit(fl!("mr-ext"), &m.rule.ext, MrField::Ext)].spacing(8),
                        row![
                            edit(fl!("mr-find"), &m.rule.find, MrField::Find),
                            edit(fl!("mr-replace"), &m.rule.replace, MrField::Replace),
                        ]
                        .spacing(8),
                        row![
                            edit(fl!("mr-start"), &m.start, MrField::Start),
                            edit(fl!("mr-step"), &m.step, MrField::Step),
                            edit(fl!("mr-digits"), &m.digits, MrField::Digits),
                        ]
                        .spacing(8),
                        row![
                            case(fl!("mr-case-keep"), Case::Keep),
                            case(fl!("mr-case-upper"), Case::Upper),
                            case(fl!("mr-case-lower"), Case::Lower),
                            case(fl!("mr-case-title"), Case::Title),
                        ]
                        .spacing(8),
                        widget::scrollable(table).height(Length::Fixed(300.0)),
                    ]
                    .spacing(12),
                )
                .primary_action(
                    widget::button::suggested(fl!("rename"))
                        .on_press_maybe(ok.then_some(Message::DialogSubmit)),
                )
                .secondary_action(cancel)
                .into()
        }
```

And a helper next to `what`:
```rust
fn problem(p: Problem) -> String {
    match p {
        Problem::BadName => fl!("mr-bad-name"),
        Problem::Duplicate => fl!("mr-duplicate"),
        Problem::Exists => fl!("mr-exists"),
    }
}
```

`Length` = `cosmic::iced::Length` (add to imports if not there). (`widget::text::caption` and `widget::text::heading` exist at this rev; `on_press_maybe` too.)

- [ ] **Step 5: i18n** — append to `en`:
```
multi-rename = Multi-rename
mr-name = Name mask
mr-ext = Extension mask
mr-find = Find
mr-replace = Replace with
mr-start = Counter start
mr-step = Step
mr-digits = Digits
mr-case-keep = As is
mr-case-upper = UPPER
mr-case-lower = lower
mr-case-title = First upper
mr-old = Old name
mr-new = New name
mr-bad-name = invalid name
mr-duplicate = same name twice
mr-exists = name is taken
```
and to `ru`:
```
multi-rename = Групповое переименование
mr-name = Маска имени
mr-ext = Маска расширения
mr-find = Найти
mr-replace = Заменить на
mr-start = Начало счётчика
mr-step = Шаг
mr-digits = Разрядов
mr-case-keep = Как есть
mr-case-upper = ЗАГЛАВНЫЕ
mr-case-lower = строчные
mr-case-title = Первая заглавная
mr-old = Было
mr-new = Станет
mr-bad-name = недопустимое имя
mr-duplicate = имя повторяется
mr-exists = имя занято
```

- [ ] **Step 6: app.rs wiring**

Imports: `use crate::dialogs::{self, Dialog, InputOp, ListItem, ListKind, MrField};` and `use shagoff_core::multirename::{self, Case, Rule};` plus `std::collections::HashSet` if not imported.

`enum Message`, after `DialogCancel,`:
```rust
    MrInput(MrField, String),
    MrCase(Case),
```

In `handle`, after the `Message::DialogInput(s) => { .. }` arm:
```rust
            Message::MrInput(field, s) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    *match field {
                        MrField::Name => &mut m.rule.name,
                        MrField::Ext => &mut m.rule.ext,
                        MrField::Find => &mut m.rule.find,
                        MrField::Replace => &mut m.rule.replace,
                        MrField::Start => &mut m.start,
                        MrField::Step => &mut m.step,
                        MrField::Digits => &mut m.digits,
                    } = s;
                }
            }
            Message::MrCase(c) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.case = c;
                }
            }
```
(Match the surrounding arms' return style: if the other arms fall through to a trailing `Task::none()`, do the same.)

In the `Message::Key` arm, inside `if let Some(d) = &self.dialog {`, before `return Task::none();`:
```rust
                    // Tab is not taken by text fields; walk the multi-rename form with it.
                    if action == Action::SwitchPane && matches!(d, Dialog::MultiRename(_)) {
                        return cosmic::iced::widget::operation::focus_next();
                    }
```

In `act`, the focus line becomes:
```rust
            let focus = matches!(d, Dialog::Mask { .. } | Dialog::Input { .. } | Dialog::MultiRename(_));
```

In `dialog_for`, before `_ => None,`:
```rust
            Action::MultiRename => {
                let wanted: HashSet<PathBuf> = panel.targets().into_iter().collect();
                // A non-UTF-8 name can't round-trip through the text masks; leave it out.
                let files: Vec<(String, SystemTime)> = panel
                    .entries()
                    .iter()
                    .filter(|e| e.os_name.to_str().is_some() && wanted.contains(&panel.cwd().join(&e.os_name)))
                    .map(|e| (e.name.clone(), e.mtime))
                    .collect();
                if files.is_empty() {
                    return None;
                }
                let dir = panel.cwd().to_path_buf();
                // Hidden files count too, even when not shown; fall back to the listing.
                let taken = multirename::other_names(&dir, &files).unwrap_or_else(|_| {
                    panel
                        .entries()
                        .iter()
                        .filter(|e| e.name != PARENT && !files.iter().any(|(n, _)| *n == e.name))
                        .map(|e| e.name.clone())
                        .collect()
                });
                let current = panel
                    .current()
                    .filter(|e| files.iter().any(|(n, _)| *n == e.name))
                    .map(|e| e.name.clone());
                Some(Dialog::MultiRename(Box::new(dialogs::MultiRename {
                    side,
                    dir,
                    files,
                    taken,
                    current,
                    rule: Rule::default(),
                    start: "1".into(),
                    step: "1".into(),
                    digits: "1".into(),
                })))
            }
```
(`SystemTime` import: `use std::time::SystemTime;` if missing.)

In `submit_dialog`, before the `d @ Dialog::List { .. }` arm:
```rust
            Dialog::MultiRename(m) => {
                let rows = m.rows(&self.tz);
                if rows.iter().any(|r| r.problem.is_some()) {
                    self.dialog = Some(Dialog::MultiRename(m)); // Enter does nothing until fixed
                    return Task::none();
                }
                let pairs = multirename::plan(&m.dir, &rows);
                if pairs.is_empty() {
                    return Task::none();
                }
                let focus = m
                    .current
                    .as_ref()
                    .and_then(|c| rows.iter().find(|r| &r.old == c))
                    .map(|r| r.new.clone());
                let job = Job::Transfer { method: Method::Move, pairs };
                self.start_job(m.side, OpKind::Move, job, focus)
            }
```

`Dialog::input_mut` needs no change (`_ => None`).

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p shagoff-commander 2>&1 | tail -20`
Expected: all tests PASS (70 old + 7 new app tests + 1 keymap test).

- [ ] **Step 8: Commit**

```bash
git add crates/app
git commit -m "feat(app): Ctrl+M multi-rename dialog with live preview

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: verify, manual run, docs

**Files:**
- Modify: `TESTING.md` (new section «Групповое переименование (10)»)
- Modify: `.claude/docs/ROADMAP.md` line 26
- Modify: `CLAUDE.md` (add `multirename` to the core module list in "Implemented so far")

- [ ] **Step 1: Full verify**

Run: `just verify 2>&1 | tail -15`
Expected: fmt clean, clippy clean, all tests pass. Fix any clippy lint in place (e.g. `large_enum_variant` is why the dialog is boxed).

- [ ] **Step 2: Manual run** — `cargo run -p shagoff-commander` in a scratch dir (`mkdir -p /tmp/claude-1000/.../scratchpad/mr && touch {a,b,c}.txt` under the session scratchpad), never in real user data. Check: Ctrl+M on 3 marked files, `[N]_[C]` with digits 3, find/replace, each case button, Tab between fields, `c` as name mask shows ⚠ and the Rename button is disabled, Enter renames, cursor follows the renamed file.

- [ ] **Step 3: TESTING.md** — add after the «Буфер обмена (09)» section, in the same table/checkbox style as that section:

```markdown
## Групповое переименование (10)

- [ ] Ctrl+M на отмеченных файлах открывает диалог, в таблице «Было → Станет» все отмеченные
- [ ] `[N]_[C]`, разрядов 3 → `a_001.txt`, `b_002.txt`…; предпросмотр меняется при каждом вводе
- [ ] Найти/заменить и четыре кнопки регистра меняют предпросмотр
- [ ] Маска, дающая одинаковые имена или имя существующего файла (в том числе скрытого), — строки с ⚠, кнопка «Переименовать» неактивна, Enter ничего не делает
- [ ] Обмен двух имён (`a`↔`b` через два прогона или цикл трёх) проходит без вопроса о замене
- [ ] Отмена посреди большого переименования: часть файлов уже переименована; при обмене файл может остаться под скрытым временным `.<имя>.<pid>.<n>.shagoff-mr` — данные целы
- [ ] Tab переходит между полями; Escape закрывает
```

(Match the exact format of the 09 section — tick boxes only after doing the check by hand.)

- [ ] **Step 4: ROADMAP** — line 26 becomes:
```markdown
- [x] Групповое переименование Ctrl+M (сделано, 10)
```

- [ ] **Step 5: CLAUDE.md** — in "Implemented so far", add `multirename` to the core module list after `mask`.

- [ ] **Step 6: Commit**

```bash
git add TESTING.md .claude/docs/ROADMAP.md CLAUDE.md
git commit -m "docs: multi-rename testing, roadmap

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
