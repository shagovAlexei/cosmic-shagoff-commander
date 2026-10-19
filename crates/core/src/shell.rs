//! The built-in terminal's shell: which ones there are, whether it waits at its prompt and how it
//! is told to follow the panel.

use crate::cmdline::quote;
use std::path::Path;

/// `/etc/shells` as a list to choose from: a name once (`/bin/zsh` and `/usr/bin/zsh` are one, the
/// first kept), restricted `rbash` and non-shells such as `tmux` and `screen` left out.
pub fn shells(etc_shells: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in etc_shells.lines().map(str::trim) {
        if !l.starts_with('/') {
            continue;
        }
        let name = Path::new(l)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(l);
        if matches!(
            name,
            "rbash" | "tmux" | "screen" | "nologin" | "false" | "git-shell"
        ) {
            continue;
        }
        if !out.iter().any(|s| s.rsplit('/').next() == Some(name)) {
            out.push(l.to_string());
        }
    }
    out
}

/// The shell to start: the chosen one, else `$SHELL`, else `/bin/sh`.
pub fn pick(chosen: &str, env_shell: Option<&str>) -> String {
    match (chosen, env_shell) {
        (c, _) if !c.is_empty() => c.to_string(),
        (_, Some(s)) if !s.is_empty() => s.to_string(),
        _ => "/bin/sh".to_string(),
    }
}

/// The shell (from its `/proc/<pid>/stat`) waits at its prompt: its process group is the
/// terminal's foreground one (`tpgid`). A program it runs takes the foreground: busy.
pub fn idle(stat: &str) -> bool {
    // "pid (comm) state ppid pgrp session tty_nr tpgid …": comm may hold spaces and parens.
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    let f: Vec<&str> = rest.split_whitespace().collect();
    match (f.get(2), f.get(5)) {
        (Some(pgrp), Some(tpgid)) => pgrp == tpgid,
        _ => false,
    }
}

/// The shell waits at its prompt and may be typed into: it holds the foreground (`idle`), and a
/// shell with a line editor (zsh, bash, fish…) has the terminal in raw mode (`canonical` false):
/// it puts it back the moment a line is accepted, before its hooks and the command run — the
/// foreground alone still looks idle then. `sh` / `dash` have no line editor: the foreground only.
pub fn at_prompt(stat: &str, canonical: Option<bool>, shell: &str) -> bool {
    let plain = matches!(
        Path::new(shell).file_name().and_then(|n| n.to_str()),
        Some("sh" | "dash" | "posh" | "ksh")
    );
    idle(stat) && (plain || canonical == Some(false))
}

/// Whether the terminal `pid`'s stdin is on is in canonical (line) mode. Opens the tty, never
/// blocks.
pub fn canonical(pid: u32) -> Option<bool> {
    use rustix::fs::{Mode, OFlags, open};
    use rustix::termios::{LocalModes, tcgetattr};
    let fd = open(
        format!("/proc/{pid}/fd/0"),
        OFlags::RDONLY | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let t = tcgetattr(&fd).ok()?;
    Some(t.local_modes.contains(LocalModes::ICANON))
}

/// What the terminal is sent to follow the panel to `dir`. First Ctrl+E Ctrl+U: a command typed
/// and not run yet goes (bash / zsh keep it for Ctrl+Y), never runs with `cd` glued to it. A
/// leading space keeps the line out of bash / zsh history (`ignorespace`); `--` and quoting take
/// any name.
pub fn cd_line(dir: &Path) -> String {
    format!("\x05\x15 cd -- {}\r", quote(&dir.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shells_once_each_without_non_shells() {
        let etc = "# /etc/shells: valid login shells\n/bin/sh\n/usr/bin/sh\n/bin/bash\n\
                   /usr/bin/bash\n/bin/rbash\n/usr/bin/dash\n/bin/zsh\n/usr/bin/zsh\n/usr/bin/tmux\n";
        assert_eq!(
            shells(etc),
            ["/bin/sh", "/bin/bash", "/usr/bin/dash", "/bin/zsh"]
        );
    }

    #[test]
    fn the_chosen_shell_then_shell_env_then_sh() {
        assert_eq!(pick("/bin/zsh", Some("/bin/bash")), "/bin/zsh");
        assert_eq!(pick("", Some("/bin/bash")), "/bin/bash");
        assert_eq!(pick("", Some("")), "/bin/sh");
        assert_eq!(pick("", None), "/bin/sh");
    }

    #[test]
    fn idle_when_the_shell_holds_the_foreground() {
        // pid (comm) state ppid pgrp session tty_nr tpgid …
        assert!(idle("4242 (zsh) S 4200 4242 4242 34817 4242 4194304 0 0"));
        assert!(!idle("4242 (zsh) S 4200 4242 4242 34817 5000 4194304 0 0")); // `vim` runs
        assert!(idle("4242 (my (odd) sh) S 4200 4242 4242 34817 4242 0")); // parens in comm
        assert!(!idle("garbage"));
    }

    #[test]
    fn at_prompt_needs_raw_mode_from_a_line_editing_shell() {
        let free = "4242 (zsh) S 4200 4242 4242 34817 4242 0";
        assert!(at_prompt(free, Some(false), "/bin/zsh"));
        // A line accepted, its hooks still running: the foreground is the shell's, the mode not.
        assert!(!at_prompt(free, Some(true), "/usr/bin/zsh"));
        assert!(!at_prompt(free, None, "/bin/bash"));
        assert!(at_prompt(free, Some(true), "/bin/dash")); // no line editor: canonical always
        let busy = "4242 (zsh) S 4200 4242 4242 34817 5000 0";
        assert!(!at_prompt(busy, Some(false), "/bin/zsh"));
    }

    #[test]
    fn cd_line_clears_the_typed_line_quotes_and_stays_out_of_history() {
        assert_eq!(
            cd_line(Path::new("/tmp/a b")),
            "\x05\x15 cd -- '/tmp/a b'\r"
        );
        let q = r#" cd -- '/x/it'\''s'"#;
        assert_eq!(cd_line(Path::new("/x/it's")), format!("\x05\x15{q}\r"));
    }
}
