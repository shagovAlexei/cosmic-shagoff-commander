//! argv for F3/F4: a configured program (no shell) with the file appended.

use std::ffi::OsString;
use std::path::Path;

pub fn command(cmd: &[String], default: &[&str], file: &Path) -> Vec<OsString> {
    let mut argv: Vec<OsString> = if cmd.is_empty() {
        default.iter().map(OsString::from).collect()
    } else {
        cmd.iter().map(OsString::from).collect()
    };
    argv.push(file.into());
    argv
}

/// Whether this process runs as root (the window opened by `as_root`, 232).
pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// argv for running `exe` as root in a window of its own on `dir` (232): pkexec clears the
/// environment, so the display goes through `env`; Wayland's socket as a full path, so root
/// needs no runtime dir (and makes no files in the user's).
pub fn as_root(exe: &Path, dir: &Path, var: impl Fn(&str) -> Option<OsString>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec!["pkexec".into(), "env".into()];
    let mut set = |k: &str, v: OsString| {
        let mut a = OsString::from(format!("{k}="));
        a.push(v);
        argv.push(a);
    };
    if let Some(w) = var("WAYLAND_DISPLAY") {
        let full = match var("XDG_RUNTIME_DIR") {
            Some(run) if Path::new(&w).is_relative() => Path::new(&run).join(&w).into(),
            _ => w,
        };
        set("WAYLAND_DISPLAY", full);
    }
    // Xlib's default cookie is the user's ~/.Xauthority; root's HOME is /root.
    let xauth = var("XAUTHORITY").or_else(|| {
        let home = var("HOME")?;
        var("DISPLAY").map(|_| Path::new(&home).join(".Xauthority").into())
    });
    if let Some(x) = xauth {
        set("XAUTHORITY", x);
    }
    let keep = [
        "DISPLAY",
        "LANG",
        "LC_ALL",
        "LC_MESSAGES",
        "LANGUAGE",
        "XDG_CURRENT_DESKTOP",
        "ICED_BACKEND",
    ];
    for k in keep {
        if let Some(v) = var(k) {
            set(k, v);
        }
    }
    argv.extend([exe.into(), dir.into()]);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_root_passes_the_display_and_a_full_wayland_socket() {
        let var = |k: &str| match k {
            "WAYLAND_DISPLAY" => Some("wayland-1".into()),
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            "LANG" => Some("ru_RU.UTF-8".into()),
            _ => None,
        };
        let argv = as_root(Path::new("/usr/bin/x"), Path::new("/a b"), var);
        let want = [
            "pkexec",
            "env",
            "WAYLAND_DISPLAY=/run/user/1000/wayland-1",
            "LANG=ru_RU.UTF-8",
            "/usr/bin/x",
            "/a b",
        ];
        assert_eq!(argv, want.map(OsString::from));
        let x11 = |k: &str| match k {
            "WAYLAND_DISPLAY" => Some("/tmp/w".into()),
            "DISPLAY" => Some(":0".into()),
            "HOME" => Some("/home/u".into()),
            _ => None,
        };
        let argv = as_root(Path::new("/x"), Path::new("/"), x11);
        assert_eq!(argv[2], "WAYLAND_DISPLAY=/tmp/w");
        assert_eq!(argv[3], "XAUTHORITY=/home/u/.Xauthority");
        assert_eq!(argv[4], "DISPLAY=:0");
    }

    #[test]
    fn empty_command_uses_default() {
        let argv = command(&[], &["xdg-open"], Path::new("/a b/f.txt"));
        assert_eq!(argv, [OsString::from("xdg-open"), "/a b/f.txt".into()]);
    }

    #[test]
    fn configured_command_keeps_its_args_and_appends_file() {
        let cmd = vec!["foot".to_string(), "-e".into(), "less".into()];
        let argv = command(&cmd, &["xdg-open"], Path::new("/f"));
        assert_eq!(argv, ["foot", "-e", "less", "/f"].map(OsString::from));
    }
}
