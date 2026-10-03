//! Alt+F7: walk a tree for names matching a TC mask and, optionally, files containing a text.
//! Synchronous with a stop flag; the app runs it on a worker thread.

use crate::mask::Mask;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Query {
    pub mask: Mask,
    /// Substring to look for inside files; `None` = names only.
    pub text: Option<String>,
    pub case_sensitive: bool,
    /// Also hidden names (and their subtrees).
    pub hidden: bool,
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
        let named = q.mask.matches(&name);
        if ft.is_dir() {
            if named && q.text.is_none() {
                found(path.clone());
            }
            let next = open(&path, dir).collect();
            stack.push(next);
        } else if named {
            let hit = match &q.text {
                None => true,
                // Through symlinks to files too; anything else (fifo, socket) is never read.
                Some(_) => {
                    fs::metadata(&path).is_ok_and(|m| m.is_file()) && contains_until(&path, q, stop)
                }
            };
            if hit {
                found(path);
            }
        }
    }
}

/// Does the file contain `q.text`? Read errors → false.
pub fn contains(path: &Path, q: &Query) -> bool {
    contains_until(path, q, &AtomicBool::new(false))
}

fn contains_until(path: &Path, q: &Query, stop: &AtomicBool) -> bool {
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
        }
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
}
