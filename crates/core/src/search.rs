//! Alt+F7: walk a tree for names matching a TC mask and, optionally, files containing a text.
//! Synchronous with a stop flag; the app runs it on a worker thread.

use crate::mask::Mask;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

pub struct Query {
    pub mask: Mask,
    /// Name as a regular expression (case as the "case sensitive" box, like masks); replaces `mask` when set.
    pub name_regex: Option<regex::Regex>,
    /// Also names inside zip / tar / 7z files (no content search there).
    pub archives: bool,
    /// Substring to look for inside files; `None` = names only.
    pub text: Option<String>,
    pub case_sensitive: bool,
    /// Also hidden names (and their subtrees).
    pub hidden: bool,
    /// Text as a regular expression (built by `regex`); replaces `text` when set.
    pub regex: Option<regex::bytes::Regex>,
    /// Size bounds in bytes, inclusive; set → files only (dirs have no size).
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Modified at or after this time.
    pub newer_than: Option<SystemTime>,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            mask: Mask::parse("*"),
            name_regex: None,
            archives: false,
            text: None,
            case_sensitive: false,
            hidden: false,
            regex: None,
            min_size: None,
            max_size: None,
            newer_than: None,
        }
    }
}

impl Query {
    fn reads_content(&self) -> bool {
        self.regex.is_some() || self.text.as_deref().is_some_and(|t| !t.is_empty())
    }

    fn filtered(&self) -> bool {
        self.min_size.is_some() || self.max_size.is_some() || self.newer_than.is_some()
    }

    fn named(&self, name: &str) -> bool {
        match &self.name_regex {
            Some(re) => re.is_match(name),
            None => self.mask.matches(name),
        }
    }

    /// Size and date filters on the entry's metadata (through symlinks).
    fn passes(&self, path: &Path, is_dir: bool) -> bool {
        if !self.filtered() {
            return true;
        }
        let Ok(m) = fs::metadata(path) else {
            return false;
        };
        // a symlink to a dir counts as a dir
        let mtime = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        self.passes_meta(is_dir || m.is_dir(), m.len(), mtime)
    }

    /// Size bounds → files only (dirs have no size); date → modified at or after.
    fn passes_meta(&self, is_dir: bool, size: u64, mtime: SystemTime) -> bool {
        let sized = self.min_size.is_some() || self.max_size.is_some();
        !(sized && is_dir)
            && self.min_size.is_none_or(|n| size >= n)
            && self.max_size.is_none_or(|n| size <= n)
            && self.newer_than.is_none_or(|t| mtime >= t)
    }

    /// Matches inside `archive`, as paths through it (`/x/a.zip/docs/f.txt` — a panel opens them).
    fn in_archive(&self, archive: &Path, stop: &AtomicBool, found: &mut dyn FnMut(PathBuf)) {
        let Ok(items) = crate::archive::members(archive, stop) else {
            return;
        };
        for (inner, dir, size, mtime) in items {
            let hidden = inner
                .components()
                .any(|c| c.as_os_str().as_encoded_bytes().starts_with(b"."));
            let name = inner.file_name().unwrap_or_default().to_string_lossy();
            if (self.hidden || !hidden) && self.named(&name) && self.passes_meta(dir, size, mtime) {
                found(archive.join(inner));
            }
        }
    }
}

/// The name field as a regular expression (Unicode, case-insensitive like masks).
pub fn name_regex(text: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(text)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| e.to_string())
}

/// The text field as a regular expression (Unicode, so `(?i)` folds Cyrillic too).
pub fn regex(text: &str, case_sensitive: bool) -> Result<regex::bytes::Regex, String> {
    regex::bytes::RegexBuilder::new(text)
        .case_insensitive(!case_sensitive)
        // `^` / `$` per line: the file is searched in windows, never as one haystack.
        .multi_line(true)
        .build()
        .map_err(|e| e.to_string())
}

const CHUNK: usize = 256 << 10;

/// Kernel/runtime trees: endless or meaningless to search.
fn skipped(p: &Path) -> bool {
    ["/proc", "/sys", "/dev", "/run"]
        .iter()
        .any(|s| p == Path::new(s))
}

/// Depth-first under `root` (names sorted per dir). `found` gets every match, `dir` every dir
/// entered. Symlinked dirs are not entered (loops). Unreadable entries are skipped. An explicit
/// stack, not recursion: a worker thread's stack must not limit the depth.
pub fn find(
    root: &Path,
    q: &Query,
    stop: &AtomicBool,
    found: &mut dyn FnMut(PathBuf),
    dir: &mut dyn FnMut(&Path),
) {
    let open = |d: &Path, dir: &mut dyn FnMut(&Path)| {
        if skipped(d) {
            return Vec::new().into_iter();
        }
        dir(d);
        let mut items: Vec<_> = fs::read_dir(d)
            .map(|rd| rd.flatten().collect())
            .unwrap_or_default();
        items.sort_by_key(|e: &fs::DirEntry| std::cmp::Reverse(e.file_name()));
        items.into_iter()
    };
    // Each level holds its remaining entries, last-sorted first so `pop` yields them in order.
    let mut stack = vec![open(root, dir).collect::<Vec<_>>()];
    while let Some(level) = stack.last_mut() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(e) = level.pop() else {
            stack.pop();
            continue;
        };
        let name = e.file_name().to_string_lossy().into_owned();
        if !q.hidden && name.starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        let named = q.named(&name);
        if ft.is_dir() {
            if named && !q.reads_content() && q.passes(&path, true) {
                found(path.clone());
            }
            let next = open(&path, dir).collect();
            stack.push(next);
        } else if named && q.passes(&path, false) {
            // Through symlinks to files too; anything else (fifo, socket) is never read.
            let hit = !q.reads_content()
                || (fs::metadata(&path).is_ok_and(|m| m.is_file())
                    && contains_until(&path, q, stop));
            if hit {
                found(path.clone());
            }
        }
        // Content search reads real files only; archive members are matched by name.
        if !ft.is_dir()
            && q.archives
            && !q.reads_content()
            && crate::archive::Format::detect(&name).is_some_and(|f| f.is_tree())
        {
            q.in_archive(&path, stop, found);
        }
    }
}

/// Does the file contain `q.text`? Read errors → false.
pub fn contains(path: &Path, q: &Query) -> bool {
    contains_until(path, q, &AtomicBool::new(false))
}

fn contains_until(path: &Path, q: &Query, stop: &AtomicBool) -> bool {
    if let Some(re) = &q.regex {
        return regex_until(path, re, stop);
    }
    let Some(text) = q.text.as_deref().filter(|t| !t.is_empty()) else {
        return true;
    };
    let needle = if q.case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    // Carry the tail of each chunk over, so a match across the boundary is still whole.
    // In source bytes: uppercase can be 3x longer than its lowercase (KELVIN SIGN → "k").
    let keep = needle.len() * 3 + 4;
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + keep);
    let mut chunk = vec![0; CHUNK];
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let n = match f.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        buf.extend_from_slice(&chunk[..n]);
        let hit = if q.case_sensitive {
            memchr::memmem::find(&buf, needle.as_bytes()).is_some()
        } else {
            String::from_utf8_lossy(&buf)
                .to_lowercase()
                .contains(&needle)
        };
        if hit {
            return true;
        }
        let cut = buf.len().saturating_sub(keep);
        buf.drain(..cut);
    }
}

/// A regex over the file in windows of whole lines (so `^` / `$` mean line starts and ends,
/// not window edges); each window keeps up to `REGEX_OVERLAP` bytes of the previous one, from a
/// line start. A match longer than that crossing a window edge is missed, and a line longer than
/// `CHUNK` is searched in pieces.
// ponytail: fixed overlap; stream the regex (regex-automata) if long multi-line matches matter.
const REGEX_OVERLAP: usize = 4096;

fn regex_until(path: &Path, re: &regex::bytes::Regex, stop: &AtomicBool) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut buf: Vec<u8> = Vec::with_capacity(2 * CHUNK);
    let mut chunk = vec![0; CHUNK];
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let n = match f.read(&mut chunk) {
            Ok(0) => return re.is_match(&buf), // the last line, without its newline
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        buf.extend_from_slice(&chunk[..n]);
        let end = match memchr::memrchr(b'\n', &buf) {
            Some(i) => i + 1,
            None if buf.len() > CHUNK => buf.len(),
            None => continue, // the line goes on: wait for its end
        };
        if re.is_match(&buf[..end]) {
            return true;
        }
        let mut cut = end.saturating_sub(REGEX_OVERLAP);
        if let Some(i) = memchr::memchr(b'\n', &buf[cut..end]) {
            cut += i + 1;
        }
        buf.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn q(mask: &str, text: Option<&str>, case: bool, hidden: bool) -> Query {
        Query {
            mask: Mask::parse(mask),
            text: text.map(Into::into),
            case_sensitive: case,
            hidden,
            ..Query::default()
        }
    }

    #[test]
    fn name_regex_replaces_the_mask() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a1.rs", "ab.rs", "Б2.txt"] {
            fs::write(d.path().join(n), "").unwrap();
        }
        let q = Query {
            name_regex: Some(name_regex(r"^\w\d\.", false).unwrap()),
            ..Query::default()
        };
        assert_eq!(rel(d.path(), run(d.path(), &q)), ["a1.rs", "Б2.txt"]);
        let q = Query {
            name_regex: Some(name_regex("^б", false).unwrap()), // case-insensitive, Cyrillic too
            ..Query::default()
        };
        assert_eq!(rel(d.path(), run(d.path(), &q)), ["Б2.txt"]);
        assert!(name_regex("(", false).is_err());
    }

    #[test]
    fn archives_searched_by_name_when_asked() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&a).unwrap());
        let opt = zip::write::SimpleFileOptions::default();
        z.add_directory("docs/", opt).unwrap();
        z.start_file("docs/x.rs", opt).unwrap();
        z.write_all(b"fn x() {}").unwrap();
        z.start_file(".hid/y.rs", opt).unwrap();
        z.start_file("big.rs", opt).unwrap();
        z.write_all(&[b'a'; 2048]).unwrap();
        z.finish().unwrap();
        let mut q = q("*.rs", None, false, false);
        assert!(run(d.path(), &q).is_empty());
        q.archives = true;
        assert_eq!(
            rel(d.path(), run(d.path(), &q)),
            ["a.zip/big.rs", "a.zip/docs/x.rs"]
        );
        q.min_size = Some(1024); // filters apply to members too
        assert_eq!(rel(d.path(), run(d.path(), &q)), ["a.zip/big.rs"]);
        q.min_size = None;
        q.hidden = true;
        assert_eq!(run(d.path(), &q).len(), 3);
        // Stop reaches into reading an archive's index
        assert!(crate::archive::members(&a, &AtomicBool::new(true)).is_err());
        // content search does not open archives
        q.text = Some("fn".into());
        assert!(run(d.path(), &q).is_empty());
    }

    fn run(root: &Path, q: &Query) -> Vec<PathBuf> {
        let mut out = Vec::new();
        find(
            root,
            q,
            &AtomicBool::new(false),
            &mut |p| out.push(p),
            &mut |_| {},
        );
        out.sort();
        out
    }

    fn rel(root: &Path, v: Vec<PathBuf>) -> Vec<String> {
        v.into_iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
            .collect()
    }

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("src/deep")).unwrap();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::write(r.join("src/main.rs"), "fn main() { println!(\"Привет\"); }").unwrap();
        fs::write(r.join("src/deep/lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(r.join("src/deep/old.bak"), "fn main").unwrap();
        fs::write(r.join("Cargo.toml"), "[package]").unwrap();
        fs::write(r.join(".git/config.rs"), "fn main").unwrap();
        d
    }

    #[test]
    fn mask_matches_nested() {
        let d = tree();
        assert_eq!(
            rel(d.path(), run(d.path(), &q("*.rs", None, false, false))),
            ["src/deep/lib.rs", "src/main.rs"]
        );
    }

    #[test]
    fn mask_exclusion() {
        let d = tree();
        let got = rel(
            d.path(),
            run(d.path(), &q("*|*.rs *.toml", None, false, false)),
        );
        assert!(got.contains(&"src/deep/old.bak".to_string()), "{got:?}");
        assert!(
            got.iter()
                .all(|p| !p.ends_with(".rs") && !p.ends_with(".toml")),
            "{got:?}"
        );
    }

    #[test]
    fn hidden_only_when_asked() {
        let d = tree();
        assert!(
            !rel(d.path(), run(d.path(), &q("*.rs", None, false, false)))
                .contains(&".git/config.rs".to_string())
        );
        assert!(
            rel(d.path(), run(d.path(), &q("*.rs", None, false, true)))
                .contains(&".git/config.rs".to_string())
        );
    }

    #[test]
    fn text_case_insensitive_cyrillic() {
        let d = tree();
        assert_eq!(
            rel(
                d.path(),
                run(d.path(), &q("*", Some("привет"), false, false))
            ),
            ["src/main.rs"]
        );
    }

    #[test]
    fn text_case_sensitive() {
        let d = tree();
        assert!(run(d.path(), &q("*", Some("FN MAIN"), true, false)).is_empty());
        assert_eq!(
            rel(
                d.path(),
                run(d.path(), &q("*", Some("fn main"), true, false))
            ),
            ["src/deep/old.bak", "src/main.rs"]
        );
    }

    #[test]
    fn text_across_chunk_boundary() {
        let d = tempfile::tempdir().unwrap();
        let mut data = vec![b'x'; CHUNK - 3];
        data.extend_from_slice(b"needle");
        data.extend(vec![b'y'; 100]);
        fs::write(d.path().join("big"), &data).unwrap();
        assert_eq!(run(d.path(), &q("*", Some("needle"), true, false)).len(), 1);
        assert_eq!(
            run(d.path(), &q("*", Some("NEEDLE"), false, false)).len(),
            1
        );
    }

    #[test]
    fn symlink_dir_loop_terminates() {
        let d = tree();
        symlink("..", d.path().join("src/loop")).unwrap();
        let got = rel(d.path(), run(d.path(), &q("*.rs", None, false, false)));
        assert_eq!(got, ["src/deep/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn stop_interrupts() {
        let d = tree();
        let stop = AtomicBool::new(false);
        let mut n = 0;
        find(
            d.path(),
            &q("*", None, false, true),
            &stop,
            &mut |_| {
                n += 1;
                stop.store(true, Ordering::Relaxed);
            },
            &mut |_| {},
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn dir_matches_without_text_only() {
        let d = tree();
        assert!(
            rel(d.path(), run(d.path(), &q("deep", None, false, false)))
                .contains(&"src/deep".to_string())
        );
        assert!(run(d.path(), &q("deep", Some("x"), false, false)).is_empty());
    }

    #[test]
    fn skips_virtual_fs_roots() {
        assert!(
            skipped(Path::new("/proc"))
                && skipped(Path::new("/sys"))
                && skipped(Path::new("/dev"))
                && skipped(Path::new("/run"))
        );
        assert!(!skipped(Path::new("/home/proc")));
    }

    #[test]
    fn regression_deep_tree_does_not_overflow_the_stack() {
        let d = tempfile::tempdir().unwrap();
        let mut p = d.path().to_path_buf();
        for _ in 0..1500 {
            p.push("a");
        }
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("deep.rs"), "").unwrap();
        // A small stack like a worker thread's: recursion per level would overflow it.
        let root = d.path().to_path_buf();
        let n = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || run(&root, &q("*.rs", None, false, false)).len())
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn regression_shrinking_lowercase_across_chunk_boundary() {
        // KELVIN SIGN (3 bytes) lowercases to "k" (1 byte): the carried tail must cover it.
        for off in 0..16 {
            let d = tempfile::tempdir().unwrap();
            let mut data = vec![b'x'; CHUNK - off];
            data.extend("\u{212A}\u{212A}\u{212A}\u{212A}".as_bytes());
            fs::write(d.path().join("f"), &data).unwrap();
            assert_eq!(
                run(d.path(), &q("*", Some("kkkk"), false, false)).len(),
                1,
                "offset {off}"
            );
        }
    }

    #[test]
    fn size_filter_keeps_files_in_range_and_drops_dirs() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("dir")).unwrap();
        fs::write(d.path().join("small"), [0u8; 10]).unwrap();
        fs::write(d.path().join("mid"), [0u8; 2000]).unwrap();
        fs::write(d.path().join("big"), [0u8; 9000]).unwrap();
        let mut query = q("*", None, false, false);
        query.min_size = Some(1000);
        query.max_size = Some(5000);
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["mid"]);
    }

    #[test]
    fn date_filter_keeps_recent() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join("old");
        fs::write(&old, "").unwrap();
        let week = std::time::Duration::from_secs(7 * 86400);
        let then = std::time::SystemTime::now() - week;
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(then)
            .unwrap();
        fs::write(d.path().join("new"), "").unwrap();
        let mut query = q("*", None, false, false);
        query.newer_than = Some(std::time::SystemTime::now() - week / 2);
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["new"]);
    }

    #[test]
    fn text_as_regex() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), "id = fooo7;").unwrap();
        fs::write(d.path().join("b"), "id = foo;").unwrap();
        fs::write(d.path().join("c"), "Привет, МИР").unwrap();
        let mut query = q("*", None, false, false);
        query.regex = Some(regex(r"fo+\d", false).unwrap());
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["a"]);
        query.regex = Some(regex("мир$", false).unwrap()); // case folds Cyrillic
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["c"]);
        query.regex = Some(regex("мир$", true).unwrap());
        assert!(run(d.path(), &query).is_empty());
        assert!(regex("(", false).is_err());
    }

    #[test]
    fn regression_regex_anchors_are_per_line_across_chunks() {
        let d = tempfile::tempdir().unwrap();
        // A chunk ends right after "мир" in the middle of a line: `мир$` must not match there.
        let mut a = "x".repeat(CHUNK - "мир".len()).into_bytes();
        a.extend("мирок\n".as_bytes());
        fs::write(d.path().join("a"), &a).unwrap();
        // After the overlap is cut, the window starts mid-line at "foo": `^foo` must not match.
        let mut b = "y".repeat(CHUNK - REGEX_OVERLAP).into_bytes();
        b.extend(format!("foo{}\n", "y".repeat(5000)).as_bytes());
        fs::write(d.path().join("b"), &b).unwrap();
        fs::write(d.path().join("c"), "first\nfoo here\nend мир\nlast").unwrap();
        let mut query = q("*", None, false, false);
        query.regex = Some(regex("мир$", false).unwrap());
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["c"]);
        query.regex = Some(regex("^foo", false).unwrap());
        assert_eq!(rel(d.path(), run(d.path(), &query)), ["c"]);
    }

    #[test]
    fn size_filter_skips_symlinked_dirs() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("dir")).unwrap();
        symlink(d.path().join("dir"), d.path().join("ln")).unwrap();
        let mut query = q("*", None, false, false);
        query.min_size = Some(0);
        assert!(run(d.path(), &query).is_empty());
    }
}
