//! Command line under the panels (TC `path>`): what a typed line means, history, inserted names.

use std::path::{Component, Path, PathBuf};

/// History length (Ctrl+E, Alt+F8).
pub const MAX_HISTORY: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    /// `cd`: change the active panel's dir (done by the app, not a shell).
    Cd(PathBuf),
    /// Anything else: run with `sh -c` in the panel's dir.
    Run(String),
}

pub fn parse(line: &str, cwd: &Path, home: &Path) -> Option<Cmd> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let arg = match line.strip_prefix("cd") {
        Some("") => return Some(Cmd::Cd(home.to_path_buf())),
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim(),
        _ => return Some(Cmd::Run(line.into())),
    };
    // `cd x && ls` and the like: the shell's business.
    if arg.contains(|c| ";&|<>$`()".contains(c)) {
        return Some(Cmd::Run(line.into()));
    }
    let arg = unquote(arg);
    let path = match arg.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => cwd.join(arg),
    };
    Some(Cmd::Cd(fold(&path)))
}

fn unquote(s: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

/// `.` and `..` resolved without touching the disk.
fn fold(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `word` as one shell word: as is when it is plain, else in single quotes.
pub fn quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+=:,@%".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// `word` at the end of `line`, one space between.
pub fn append(line: &str, word: &str) -> String {
    if line.is_empty() || line.ends_with(' ') {
        format!("{line}{word}")
    } else {
        format!("{line} {word}")
    }
}

/// History with `line` on top, no repeats, at most `MAX_HISTORY`.
pub fn remember(list: &[String], line: &str) -> Vec<String> {
    let line = line.trim();
    if line.is_empty() {
        return list.to_vec();
    }
    std::iter::once(line.to_string())
        .chain(list.iter().filter(|s| *s != line).cloned())
        .take(MAX_HISTORY)
        .collect()
}

/// The entry after `line` (newer), stopping at the newest; a line not in the list gets the newest.
pub fn next(list: &[String], line: &str) -> Option<String> {
    match list.iter().position(|s| s == line) {
        Some(0) => None,
        Some(i) => list.get(i - 1).cloned(),
        None => list.first().cloned(),
    }
}

/// Ctrl+E: the command before `line` in the history; from the oldest (or a typed line) back to the newest.
pub fn previous(list: &[String], line: &str) -> Option<String> {
    let next = match list.iter().position(|s| s == line) {
        Some(i) if i + 1 < list.len() => i + 1,
        _ => 0,
    };
    list.get(next).cloned()
}

/// The flag after which a terminal runs a command: `--` for gnome-terminal / kgx, `-e` for the
/// rest (cosmic-term, xterm, konsole, alacritty, foot -e is fine too).
fn exec_flag(prog: &str) -> &'static str {
    let name = Path::new(prog)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(prog);
    match name {
        "gnome-terminal" | "kgx" | "ptyxis" => "--",
        _ => "-e",
    }
}

/// What runs `line`: `sh -c` (no window), or in `terminal` (empty → `cosmic-term -e`) with a shell
/// left open after it (Shift+Enter, TC `cmd /k`). The terminal is told the dir too: its profile
/// may have its own.
pub fn argv(line: &str, dir: &Path, terminal: Option<&[String]>) -> Vec<String> {
    let Some(term) = terminal else {
        return ["sh", "-c", line].map(String::from).to_vec();
    };
    let term: Vec<String> = match term {
        [] => vec!["cosmic-term".into(), "-e".into()],
        // Just the program (`["cosmic-term"]`): without its "run this" flag it ignores the
        // command and opens a plain shell.
        [prog] => vec![prog.clone(), exec_flag(prog).into()],
        _ => term.to_vec(),
    };
    let dir = quote(&dir.display().to_string());
    // Lines, not `;`: a `#` in the command must not comment out the shell that keeps it open.
    let keep = format!("cd {dir} || exit\n{line}\nexec \"${{SHELL:-sh}}\"");
    term.iter()
        .cloned()
        .chain(["sh".into(), "-c".into(), keep])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd(line: &str) -> Option<Cmd> {
        parse(line, Path::new("/a/b"), Path::new("/home/u"))
    }

    #[test]
    fn empty_line_is_nothing() {
        assert_eq!(cd(""), None);
        assert_eq!(cd("   "), None);
    }

    #[test]
    fn cd_alone_goes_home() {
        assert_eq!(cd("cd"), Some(Cmd::Cd("/home/u".into())));
        assert_eq!(cd("  cd  "), Some(Cmd::Cd("/home/u".into())));
    }

    #[test]
    fn cd_paths() {
        assert_eq!(cd("cd x/y"), Some(Cmd::Cd("/a/b/x/y".into())));
        assert_eq!(cd("cd .."), Some(Cmd::Cd("/a".into())));
        assert_eq!(cd("cd ../../.."), Some(Cmd::Cd("/".into())));
        assert_eq!(cd("cd ./c/../d"), Some(Cmd::Cd("/a/b/d".into())));
        assert_eq!(cd("cd /etc"), Some(Cmd::Cd("/etc".into())));
        assert_eq!(cd("cd ~"), Some(Cmd::Cd("/home/u".into())));
        assert_eq!(cd("cd ~/doc"), Some(Cmd::Cd("/home/u/doc".into())));
        assert_eq!(cd("cd \"my dir\""), Some(Cmd::Cd("/a/b/my dir".into())));
        assert_eq!(cd("cd 'my dir'"), Some(Cmd::Cd("/a/b/my dir".into())));
        assert_eq!(cd("cd my dir"), Some(Cmd::Cd("/a/b/my dir".into())));
    }

    #[test]
    fn other_lines_run_as_typed() {
        assert_eq!(cd("ls -la"), Some(Cmd::Run("ls -la".into())));
        assert_eq!(cd("  make  x "), Some(Cmd::Run("make  x".into())));
        // Not `cd`: a program whose name starts with it.
        assert_eq!(cd("cdrecord x"), Some(Cmd::Run("cdrecord x".into())));
        assert_eq!(cd("cd x && ls"), Some(Cmd::Run("cd x && ls".into())));
    }

    #[test]
    fn quote_only_when_needed() {
        assert_eq!(quote("file.txt"), "file.txt");
        assert_eq!(quote("/a/b-c_d+1.txt"), "/a/b-c_d+1.txt");
        assert_eq!(quote("Мои файлы"), "'Мои файлы'");
        assert_eq!(quote("a$b"), "'a$b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn append_separates_with_one_space() {
        assert_eq!(append("", "x"), "x");
        assert_eq!(append("ls", "x"), "ls x");
        assert_eq!(append("ls ", "x"), "ls x");
    }

    #[test]
    fn remember_puts_last_first_without_repeats() {
        let list = remember(&["b".into(), "a".into()], "a");
        assert_eq!(list, ["a", "b"]);
        assert_eq!(remember(&list, "  "), list);
        let long: Vec<String> = (0..MAX_HISTORY).map(|i| i.to_string()).collect();
        let list = remember(&long, "new");
        assert_eq!(list.len(), MAX_HISTORY);
        assert_eq!(list[0], "new");
        assert_eq!(list[MAX_HISTORY - 1], (MAX_HISTORY - 2).to_string());
    }

    #[test]
    fn previous_steps_back_in_a_circle() {
        let h: Vec<String> = vec!["c".into(), "b".into(), "a".into()];
        assert_eq!(previous(&h, ""), Some("c".into()));
        assert_eq!(previous(&h, "c"), Some("b".into()));
        assert_eq!(previous(&h, "a"), Some("c".into()));
        assert_eq!(previous(&h, "typed"), Some("c".into()));
        assert_eq!(previous(&[], ""), None);
    }

    #[test]
    fn next_steps_forward_to_the_newest() {
        let h: Vec<String> = vec!["c".into(), "b".into(), "a".into()];
        assert_eq!(next(&h, "a"), Some("b".into()));
        assert_eq!(next(&h, "b"), Some("c".into()));
        assert_eq!(next(&h, "c"), None);
        assert_eq!(next(&h, "typed"), Some("c".into()));
    }

    #[test]
    fn argv_runs_through_sh_or_a_terminal_that_stays_open() {
        let dir = Path::new("/my dir");
        assert_eq!(argv("ls -l", dir, None), ["sh", "-c", "ls -l"]);
        let term = ["foot".to_string(), "-e".into()];
        assert_eq!(
            argv("make", dir, Some(&term)),
            [
                "foot",
                "-e",
                "sh",
                "-c",
                "cd '/my dir' || exit\nmake\nexec \"${SHELL:-sh}\""
            ]
        );
        assert_eq!(argv("x", dir, Some(&[]))[..3], ["cosmic-term", "-e", "sh"]);
        // regression: `terminal = ["cosmic-term"]` opened a bare shell, the command was ignored
        let only = |p: &str| argv("x", dir, Some(&[p.to_string()]))[..3].to_vec();
        assert_eq!(only("cosmic-term"), ["cosmic-term", "-e", "sh"]);
        assert_eq!(
            only("/usr/bin/gnome-terminal"),
            ["/usr/bin/gnome-terminal", "--", "sh"]
        );
    }
}
