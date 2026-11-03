//! Running as a flatpak (218): every program we start runs on the host through `flatpak-spawn
//! --host`, the host's `/etc` and `/usr` are read under `/run/host`, its mounts through `cat`.
//! Outside a flatpak each of these is the plain thing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Where the host's `/etc` and `/usr` are inside the sandbox (`host-etc`, `host-os`).
const HOST: &str = "/run/host";

/// Whether we run inside a flatpak sandbox.
pub fn inside() -> bool {
    static INSIDE: OnceLock<bool> = OnceLock::new();
    *INSIDE.get_or_init(|| Path::new("/.flatpak-info").exists())
}

/// A command for the program `prog` on the host. Environment set on it does not reach the host
/// program inside: pass it as `env K=V prog …` (`command("env")`) instead.
pub fn command(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    let argv = argv(inside(), prog.as_ref());
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    c
}

/// The command line `argv` on the host, in `dir`. Inside, an argument or dir under `/run/host`
/// (a panel in the host's `/etc`, `/usr`) is passed as the host's own path; the dir goes as
/// `--directory` (flatpak-spawn would send its own cwd, which need not exist on the host).
/// ponytail: a path inside a longer argument (`sh -c "… /run/host/etc/x"`) is not mapped.
pub fn command_in<S: AsRef<std::ffi::OsStr>>(argv: &[S], dir: Option<&Path>) -> Command {
    let (prog, args) = match argv.split_first() {
        Some((p, a)) => (p.as_ref(), a),
        None => ("".as_ref(), &[][..]),
    };
    let host = |a: &std::ffi::OsStr| outside(Path::new(a)).into_os_string();
    let mut c = if inside() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host");
        if let Some(d) = dir {
            c.arg(format!("--directory={}", outside(d).display()));
        }
        c.arg(host(prog));
        c
    } else {
        let mut c = Command::new(prog);
        if let Some(d) = dir {
            c.current_dir(d);
        }
        c
    };
    c.args(args.iter().map(|a| host(a.as_ref())));
    c
}

fn argv(inside: bool, prog: &std::ffi::OsStr) -> Vec<std::ffi::OsString> {
    if inside {
        vec!["flatpak-spawn".into(), "--host".into(), prog.into()]
    } else {
        vec![prog.into()]
    }
}

/// The host's own dirs, under `/run/host` inside (`host-os`, `host-etc`; the rest are links).
const SYSTEM: [&str; 7] = ["etc", "usr", "bin", "sbin", "lib", "lib32", "lib64"];

/// The host's file `p` (`/etc/passwd`, `/usr/share/…`) as we can read it.
pub fn path(p: &Path) -> PathBuf {
    path_in(inside(), p)
}

fn path_in(inside: bool, p: &Path) -> PathBuf {
    let rel = p.strip_prefix("/").unwrap_or(p);
    let system =
        (rel.components().next()).is_some_and(|c| SYSTEM.iter().any(|s| c.as_os_str() == *s));
    if inside && system {
        Path::new(HOST).join(rel)
    } else {
        p.to_path_buf()
    }
}

/// `p` as the host sees it: a path we read under `/run/host` handed to a host program.
pub fn outside(p: &Path) -> PathBuf {
    outside_in(inside(), p)
}

fn outside_in(inside: bool, p: &Path) -> PathBuf {
    match p.strip_prefix(HOST) {
        Ok(rest) if inside => Path::new("/").join(rest),
        _ => p.to_path_buf(),
    }
}

/// `prog` (a name on `$PATH`, or a path) on the host: its path as the host sees it. Inside,
/// `$PATH` is the host's (`flatpak/launch.sh`).
pub fn which(prog: &str) -> Option<PathBuf> {
    let found = |p: PathBuf| path(&p).is_file().then_some(p);
    if prog.contains('/') {
        return found(prog.into());
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|d| found(d.join(prog)))
}

/// `p` with its links resolved, as the host sees it.
pub fn canonicalize(p: &Path) -> Option<PathBuf> {
    path(p).canonicalize().ok().map(|c| outside(&c))
}

/// The host's mount table (`/proc/self/mounts`); the sandbox's own is not it.
/// Inside it is a host process: kept for a second, the drive buttons ask for it on the UI thread.
pub fn mounts() -> String {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<(Instant, String)>> = Mutex::new(None);
    if !inside() {
        return std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    }
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, m)) = &*last
        && at.elapsed() < Duration::from_secs(1)
    {
        return m.clone();
    }
    let m = command("cat")
        .arg("/proc/self/mounts")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    *last = Some((Instant::now(), m.clone()));
    m
}

/// Stops the process group `group` we started. Inside, our child is `flatpak-spawn`: it passes
/// TERM on to the host program, KILL would only kill itself.
pub fn kill_group(group: rustix::process::Pid) {
    use rustix::process::{Signal, kill_process_group};
    let sig = if inside() { Signal::TERM } else { Signal::KILL };
    let _ = kill_process_group(group, sig);
}

/// The built-in terminal's shell inside: `flatpak-spawn --host SHELL` would get no controlling
/// tty (our pty is the sandbox's: no job control, `tty` fails), so `script` runs it on a pty of
/// the host's, its pid written to `pidfile` (the follow and resize checks ask the host about it).
/// `TERM` is passed by hand: the host program gets the session's environment, not ours.
/// The shell reaches `script`'s own `$SHELL -c` through the environment, never quoted into it.
pub fn shell_argv(shell: &str, pidfile: &Path, dir: Option<&Path>) -> (String, Vec<String>) {
    let script =
        r#"echo $$ > "$1"; SHAGOFF_SHELL=$2 exec script -qfec 'exec "$SHAGOFF_SHELL"' /dev/null"#;
    let mut args: Vec<String> = [
        "--host",
        "--env=TERM=xterm-256color",
        "--env=COLORTERM=truecolor",
    ]
    .map(String::from)
    .into();
    args.extend(dir.map(|d| format!("--directory={}", outside(d).display())));
    let rest = ["sh", "-c", script, "sh", &pidfile.to_string_lossy(), shell];
    args.extend(rest.map(String::from));
    ("flatpak-spawn".into(), args)
}

/// `script`'s pid on the host, from `shell_argv`'s pid file.
fn script_pid(pidfile: &Path) -> Option<u32> {
    std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()
}

/// The host shell's `/proc/PID/stat` and whether its tty is in canonical mode
/// (`shell::at_prompt`'s input), asked of the host.
pub fn shell_state(pidfile: &Path) -> Option<(String, Option<bool>)> {
    let pid = script_pid(pidfile)?.to_string();
    let ask = r#"c=$(pgrep -P "$1" | head -n1) && cat /proc/$c/stat && stty -a < /proc/$c/fd/0"#;
    let out = command("sh").args(["-c", ask, "sh", &pid]).output().ok()?;
    let out = String::from_utf8_lossy(&out.stdout);
    let (stat, modes) = out.split_once('\n')?;
    let canonical = modes.split_whitespace().find_map(|w| match w {
        "icanon" => Some(true),
        "-icanon" => Some(false),
        _ => None,
    });
    Some((stat.to_string(), canonical))
}

/// The window was resized: `flatpak-spawn` does not pass SIGWINCH on, `script` is told by hand
/// (it then reads the new size off our pty). Not waited for.
/// One at a time: a window dragged resizes it many times a second, one `kill` covers them all.
pub fn winch(pidfile: &Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PENDING: AtomicBool = AtomicBool::new(false);
    let Some(pid) = script_pid(pidfile) else {
        return;
    };
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        // After the pty took the size (`on_resize` goes through the terminal's event loop).
        std::thread::sleep(std::time::Duration::from_millis(100));
        PENDING.store(false, Ordering::Release);
        let _ = command("kill").args(["-WINCH", &pid.to_string()]).status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_programs_run_through_flatpak_spawn() {
        assert_eq!(argv(false, "gio".as_ref()), ["gio"]);
        assert_eq!(
            argv(true, "gio".as_ref()),
            ["flatpak-spawn", "--host", "gio"]
        );
    }

    #[test]
    fn the_shell_runs_under_script_with_its_pid_written() {
        let (prog, args) = shell_argv("/bin/zsh", Path::new("/tmp/p"), None);
        assert_eq!(prog, "flatpak-spawn");
        assert_eq!(args[0], "--host");
        assert_eq!(args[args.len() - 3..], ["sh", "/tmp/p", "/bin/zsh"]);
        // The same script, run here: the pid file, then the shell under `script`.
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("pid");
        let at = args.iter().position(|a| a == "-c").unwrap();
        // A shell path the host's `sh -c` would expand if it were quoted into the command.
        let shell = d.path().join("odd \"$x` sh");
        std::fs::write(&shell, "#!/bin/sh\necho started\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = Command::new("sh")
            .args(&args[at..at + 3])
            .arg(&pidfile)
            .arg(&shell)
            .output()
            .unwrap();
        assert!(script_pid(&pidfile).is_some());
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("started"),
            "{out:?}"
        );
    }

    #[test]
    fn host_paths_are_passed_as_the_hosts_own() {
        // Outside a flatpak nothing changes (`outside` maps only inside).
        let c = command_in(&["xdg-open", "/run/host/etc/x"], Some(Path::new("/tmp")));
        assert_eq!(c.get_program(), "xdg-open");
        assert_eq!(c.get_current_dir(), Some(Path::new("/tmp")));
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(outside_in(true, &p("/run/host/etc/x")), p("/etc/x"));
        assert_eq!(outside_in(true, &p("/run/hostile")), p("/run/hostile"));
        assert_eq!(
            outside_in(false, &p("/run/host/etc/x")),
            p("/run/host/etc/x")
        );
    }

    #[test]
    fn inside_etc_and_usr_are_read_under_run_host() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(path_in(true, &p("/etc/passwd")), p("/run/host/etc/passwd"));
        assert_eq!(
            path_in(true, &p("/usr/share/x")),
            p("/run/host/usr/share/x")
        );
        assert_eq!(path_in(true, &p("/home/a")), p("/home/a"));
        assert_eq!(path_in(true, &p("/bin/sh")), p("/run/host/bin/sh"));
        assert_eq!(path_in(true, &p("/etcetera")), p("/etcetera"));
        assert_eq!(path_in(false, &p("/etc/passwd")), p("/etc/passwd"));
    }
}
