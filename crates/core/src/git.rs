//! Git in the panel (226): the branch of the dir's working copy and each item's state, read by
//! `git status` alongside the dir's own scan.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// An item's state; a dir takes the strongest of what is inside it (the order here).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Ignored,
    /// Untracked, or added to the index.
    New,
    Changed,
    Conflict,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    /// `HEAD` when detached.
    pub branch: String,
    states: HashMap<OsString, State>,
    /// The dir itself is untracked or ignored: so is everything in it.
    all: Option<State>,
}

impl Repo {
    /// The state of the item `name` of the dir; `None`: unchanged (or not known to git).
    pub fn state(&self, name: &OsStr) -> Option<State> {
        self.states.get(name).copied().or(self.all)
    }
}

/// The working copy `dir` is in, read by `git status`; `None`: not in one, no git, or it failed.
/// ponytail: no time limit — a huge repo on a network mount holds the scan as long as git takes.
pub fn read(dir: &Path) -> Option<Repo> {
    let real = std::fs::canonicalize(dir).ok()?;
    let root = real
        .ancestors()
        .find(|a| a.join(".git").symlink_metadata().is_ok())?;
    let mut prefix = real
        .strip_prefix(root)
        .ok()?
        .as_os_str()
        .as_bytes()
        .to_vec();
    if !prefix.is_empty() {
        prefix.push(b'/');
    }
    let out = crate::host::command("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(crate::host::outside(&real))
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--ignored",
            "--",
            ".",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse(&out.stdout, &prefix))
}

/// `git status --porcelain=v1 -z --branch` output; its paths are from the repo's root, `prefix`
/// is the dir's own (`a/b/`, empty at the root).
pub fn parse(out: &[u8], prefix: &[u8]) -> Repo {
    let mut repo = Repo::default();
    let mut fields = out.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        if let Some(b) = f.strip_prefix(b"## ") {
            repo.branch = branch(&String::from_utf8_lossy(b));
            continue;
        }
        if f.len() < 4 {
            continue;
        }
        let (x, y, path) = (f[0], f[1], &f[3..]);
        // A rename / copy: its old path follows.
        if matches!(x, b'R' | b'C') {
            fields.next();
        }
        let state = match (x, y) {
            (b'?', b'?') => State::New,
            (b'!', b'!') => State::Ignored,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => State::Conflict,
            (b'A', _) => State::New,
            _ => State::Changed,
        };
        // An untracked / ignored dir the panel's dir is in.
        if path.ends_with(b"/") && prefix.starts_with(path) {
            repo.all = Some(state);
            continue;
        }
        let Some(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        let (name, inner) = match rest.iter().position(|&b| b == b'/') {
            Some(i) => (&rest[..i], i + 1 < rest.len()),
            None => (rest, false),
        };
        // An ignored file deep inside does not make its dir ignored.
        if name.is_empty() || (inner && state == State::Ignored) {
            continue;
        }
        let s = repo
            .states
            .entry(OsStr::from_bytes(name).to_os_string())
            .or_insert(state);
        *s = (*s).max(state);
    }
    repo
}

/// The branch from the `## …` line: `main...origin/main [ahead 1]`, `No commits yet on main`,
/// `HEAD (no branch)`.
fn branch(line: &str) -> String {
    let line = ["No commits yet on ", "Initial commit on "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .unwrap_or(line);
    let line = line.split("...").next().unwrap_or(line);
    line.split(' ').next().unwrap_or(line).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(r: &Repo, name: &str) -> Option<State> {
        r.state(OsStr::new(name))
    }

    #[test]
    fn states_by_name_at_the_root() {
        let out =
            b"## main...origin/main [ahead 1]\0 M a.rs\0?? new.txt\0!! target/\0A  added.rs\0\
UU both.rs\0R  moved.rs\0old.rs\0 M src/lib.rs\0?? src/x.rs\0?? docs/\0!! src/gen.o\0";
        let r = parse(out, b"");
        assert_eq!(r.branch, "main");
        assert_eq!(st(&r, "a.rs"), Some(State::Changed));
        assert_eq!(st(&r, "new.txt"), Some(State::New));
        assert_eq!(st(&r, "target"), Some(State::Ignored));
        assert_eq!(st(&r, "added.rs"), Some(State::New));
        assert_eq!(st(&r, "both.rs"), Some(State::Conflict));
        assert_eq!(st(&r, "moved.rs"), Some(State::Changed));
        // The old name of a rename is not an item.
        assert_eq!(st(&r, "old.rs"), None);
        // A dir: the strongest inside; an ignored file inside does not count.
        assert_eq!(st(&r, "src"), Some(State::Changed));
        assert_eq!(st(&r, "docs"), Some(State::New));
        assert_eq!(st(&r, "clean.rs"), None);
    }

    #[test]
    fn states_in_a_subdir() {
        let out = b"## No commits yet on dev\0?? src/a.rs\0 M top.rs\0!! src/b/\0?? src/c/d/e\0";
        let r = parse(out, b"src/");
        assert_eq!(r.branch, "dev");
        assert_eq!(st(&r, "a.rs"), Some(State::New));
        assert_eq!(st(&r, "top.rs"), None);
        assert_eq!(st(&r, "b"), Some(State::Ignored));
        assert_eq!(st(&r, "c"), Some(State::New));
    }

    #[test]
    fn inside_an_ignored_dir_everything_is() {
        let r = parse(b"## HEAD (no branch)\0!! target/\0", b"target/debug/");
        assert_eq!(r.branch, "HEAD");
        assert_eq!(st(&r, "anything"), Some(State::Ignored));
    }

    #[test]
    fn read_a_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(d)
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q", "-b", "trunk"]) {
            return; // no git here
        }
        assert!(read(d).is_some_and(|r| r.branch == "trunk"));
        std::fs::create_dir(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/f"), "x").unwrap();
        std::fs::write(d.join(".gitignore"), "*.o\n").unwrap();
        std::fs::write(d.join("sub/x.o"), "").unwrap();
        let r = read(d).unwrap();
        assert_eq!(st(&r, "sub"), Some(State::New));
        let r = read(&d.join("sub")).unwrap();
        assert_eq!(st(&r, "f"), Some(State::New));
        assert_eq!(st(&r, "x.o"), Some(State::Ignored));
        // Not a working copy.
        let other = tempfile::tempdir().unwrap();
        assert_eq!(read(other.path()), None);
    }
}
