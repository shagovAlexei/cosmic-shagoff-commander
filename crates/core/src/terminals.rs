//! Terminal programs: the installed ones (`.desktop` files in the `TerminalEmulator` category),
//! the system's default and how each is told to run a command.

use crate::openwith::{data_dirs, parse_desktop, parse_mime};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    /// The `.desktop` file name (`com.system76.CosmicTerm.desktop`): what the config keeps.
    pub id: String,
    pub name: String,
    /// `Exec=` split, field codes (`%U`…) dropped: opens a window with a shell.
    pub argv: Vec<String>,
}

impl Terminal {
    /// What runs a command in it: `argv`, then its "run this" flag (none for kitty / foot, which
    /// take the command as their arguments).
    pub fn run_prefix(&self) -> Vec<String> {
        let flag = run_flag(self.argv.first().map_or("", String::as_str));
        self.argv
            .iter()
            .cloned()
            .chain(flag.map(String::from))
            .collect()
    }
}

/// The flag after which terminal `prog` (a path or a name) runs a command: `--` for
/// gnome-terminal and the like, `-x` for xfce4-terminal / terminator (their `-e` takes one
/// string), none for kitty / foot (the command is their arguments), else `-e`.
pub fn run_flag(prog: &str) -> Option<&'static str> {
    match program(prog) {
        "gnome-terminal" | "kgx" | "ptyxis" | "wezterm" => Some("--"),
        "xfce4-terminal" | "terminator" => Some("-x"),
        "kitty" | "foot" | "footclient" => None,
        _ => Some("-e"),
    }
}

/// The file name of a program path (`/usr/bin/foot` → `foot`).
fn program(p: &str) -> &str {
    Path::new(p)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(p)
}

/// `Exec=` and whether `Categories=` has `TerminalEmulator`, from `[Desktop Entry]`.
fn exec_and_terminal(text: &str) -> (Option<String>, bool) {
    let (mut exec, mut term, mut in_entry) = (None, false, false);
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        match l.split_once('=').filter(|_| in_entry) {
            Some(("Exec", v)) => exec = Some(v.trim().to_string()),
            Some(("Categories", v)) => term = v.split(';').any(|c| c == "TerminalEmulator"),
            _ => {}
        }
    }
    (exec, term)
}

/// `Exec=` as arguments (the Desktop Entry quoting: `"…"` with `\"` `\\` inside), field codes
/// (`%f %U %i %c %k`…) dropped, `%%` → `%`.
pub fn exec_argv(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => (quoted, any) = (!quoted, true),
            '\\' if quoted => cur.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if any {
                    args.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => (cur.push(c), any = true).1,
        }
    }
    if any {
        args.push(cur);
    }
    args.into_iter()
        .filter(|a| !(a.len() == 2 && a.starts_with('%') && a != "%%"))
        .map(|a| a.replace("%%", "%"))
        .collect()
}

/// The terminals in `dirs` (`openwith::data_dirs()`), by name. An id in an earlier dir hides later
/// ones; hidden entries (`NoDisplay`) and servers (`foot --server`) are left out.
pub fn installed(dirs: &[PathBuf], lang: &str) -> Vec<Terminal> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Terminal> = (dirs.iter())
        .filter_map(|d| std::fs::read_dir(d.join("applications")).ok())
        .flat_map(|rd| rd.filter_map(Result::ok).map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
        .filter(|p| seen.insert(p.file_name().map(ToOwned::to_owned)))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let (name, _, shown) = parse_desktop(&text, lang)?;
            let (Some(exec), true) = exec_and_terminal(&text) else {
                return None;
            };
            let argv = exec_argv(&exec);
            (shown && !argv.is_empty() && !argv.iter().any(|a| a == "--server")).then_some(())?;
            Some(Terminal {
                id: p.file_name()?.to_string_lossy().into_owned(),
                name,
                argv,
            })
        })
        .collect();
    out.sort_by_cached_key(|t| t.name.to_lowercase());
    out
}

/// The system's terminal among `terms`: the desktop's choice (`gio mime
/// x-scheme-handler/terminal` output, COSMIC's «Default applications»), else what
/// `x-terminal-emulator` points to (`real`: its resolved path), else the first.
pub fn default_of<'a>(
    terms: &'a [Terminal],
    gio_mime: Option<&str>,
    real: Option<&Path>,
) -> Option<&'a Terminal> {
    let by_id = gio_mime
        .and_then(|out| parse_mime(out).into_iter().next())
        .and_then(|id| terms.iter().find(|t| t.id == id));
    let by_alt = || {
        let name = real?.file_name()?.to_str()?;
        terms
            .iter()
            .find(|t| t.argv.first().is_some_and(|p| program(p) == name))
    };
    by_id.or_else(by_alt).or(terms.first())
}

/// The installed terminals and the default's index. Blocking: reads the `.desktop` files, runs
/// `gio`.
pub fn scan(lang: &str) -> (Vec<Terminal>, Option<usize>) {
    let terms = installed(&data_dirs(), lang);
    let gio = std::process::Command::new("gio")
        .args(["mime", "x-scheme-handler/terminal"])
        .env("LC_ALL", "C")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let real = std::fs::canonicalize("/usr/bin/x-terminal-emulator").ok();
    let default = default_of(&terms, gio.as_deref(), real.as_deref())
        .and_then(|d| terms.iter().position(|t| t == d));
    (terms, default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, argv: &[&str]) -> Terminal {
        Terminal {
            id: id.into(),
            name: id.into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn exec_lines_split_with_field_codes_dropped() {
        assert_eq!(exec_argv("alacritty"), ["alacritty"]);
        assert_eq!(
            exec_argv("/usr/bin/ghostty --gtk-single-instance=true %U"),
            ["/usr/bin/ghostty", "--gtk-single-instance=true"]
        );
        assert_eq!(
            exec_argv(r#""/opt/My Term/term" --title "a \"b\"" 100%%"#),
            ["/opt/My Term/term", "--title", "a \"b\"", "100%"]
        );
        assert_eq!(exec_argv(r#"term """#), ["term", ""]); // an empty quoted argument stays
    }

    #[test]
    fn each_terminal_gets_its_run_flag() {
        let p = |argv: &[&str]| t("x", argv).run_prefix();
        assert_eq!(p(&["cosmic-term"]), ["cosmic-term", "-e"]);
        assert_eq!(
            p(&["/usr/bin/gnome-terminal"]),
            ["/usr/bin/gnome-terminal", "--"]
        );
        assert_eq!(
            p(&["wezterm", "start", "--cwd", "."]),
            ["wezterm", "start", "--cwd", ".", "--"]
        );
        assert_eq!(p(&["xfce4-terminal"]), ["xfce4-terminal", "-x"]);
        assert_eq!(
            p(&["/home/u/.local/kitty.app/bin/kitty"]),
            ["/home/u/.local/kitty.app/bin/kitty"]
        );
        assert_eq!(p(&["foot"]), ["foot"]);
        assert_eq!(p(&["something-new"]), ["something-new", "-e"]);
    }

    #[test]
    fn default_is_the_desktops_then_the_alternative_then_the_first() {
        let terms = [
            t("Alacritty.desktop", &["alacritty"]),
            t("com.system76.CosmicTerm.desktop", &["cosmic-term"]),
            t("com.mitchellh.ghostty.desktop", &["/usr/bin/ghostty"]),
        ];
        let gio = "Default application for “x-scheme-handler/terminal”: com.system76.CosmicTerm.desktop\n";
        let ghostty = Path::new("/usr/bin/ghostty");
        assert_eq!(
            default_of(&terms, Some(gio), Some(ghostty)),
            Some(&terms[1])
        );
        let none = "No default applications for “x-scheme-handler/terminal”\n";
        assert_eq!(
            default_of(&terms, Some(none), Some(ghostty)),
            Some(&terms[2])
        );
        assert_eq!(default_of(&terms, None, None), Some(&terms[0]));
        assert_eq!(default_of(&[], None, None), None);
    }

    #[test]
    fn installed_reads_terminal_entries_only() {
        let d = tempfile::tempdir().unwrap();
        let apps = d.path().join("applications");
        std::fs::create_dir(&apps).unwrap();
        let entry = |file: &str, body: &str| {
            std::fs::write(
                apps.join(file),
                format!("[Desktop Entry]\nType=Application\n{body}"),
            )
            .unwrap()
        };
        entry(
            "foot.desktop",
            "Name=Foot\nExec=foot\nCategories=System;TerminalEmulator;\n",
        );
        entry(
            "foot-server.desktop",
            "Name=Foot Server\nExec=foot --server\nCategories=TerminalEmulator;\n",
        );
        entry(
            "kitty-open.desktop",
            "Name=Kitty URL\nExec=kitty +open %U\nCategories=TerminalEmulator;\nNoDisplay=true\n",
        );
        entry(
            "editor.desktop",
            "Name=Editor\nExec=ed\nCategories=Utility;\n",
        );
        entry(
            "Alacritty.desktop",
            "Name=Alacritty\nExec=alacritty\nCategories=System;TerminalEmulator;\n[Desktop Action New]\nExec=alacritty --new\n",
        );
        let got = installed(&[d.path().to_path_buf()], "en");
        let ids: Vec<&str> = got.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["Alacritty.desktop", "foot.desktop"]);
        assert_eq!(got[0].argv, ["alacritty"]); // not the action's Exec
    }
}
