//! The desktop's thumbnailers (freedesktop: `thumbnailers/*.thumbnailer` in the data dirs) for
//! the Thumbnails view's files we can't decode ourselves: PDF, video, SVG, fonts…
//! The file's type comes from its name, by shared-mime-info's `mime/globs2`.
// ponytail: by name only (no magic sniffing, no mime aliases / subclasses); add if types go missing.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub struct Thumbnailer {
    pub exec: String,
    pub mimes: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Registry {
    list: Vec<Thumbnailer>,
    /// Name suffix (`pdf`, `tar.gz`; lowercase unless the glob is case-sensitive, `cs`) →
    /// (weight, mime type), the heaviest glob of all the dirs kept.
    globs: HashMap<String, (u32, String)>,
    cs_globs: HashMap<String, (u32, String)>,
}

impl Registry {
    /// From `dirs` (`openwith::data_dirs()`): an earlier dir's thumbnailer of a name hides later
    /// ones, a thumbnailer whose `TryExec` isn't installed is left out.
    pub fn load(dirs: &[PathBuf]) -> Self {
        let mut r = Self::default();
        let mut seen = std::collections::HashSet::new();
        for d in dirs {
            if let Ok(text) = std::fs::read_to_string(d.join("mime/globs2")) {
                r.add_globs(&text);
            }
            let Ok(rd) = std::fs::read_dir(d.join("thumbnailers")) else {
                continue;
            };
            let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            files.sort();
            for f in files {
                let Some(name) = f.file_name().map(OsString::from) else {
                    continue;
                };
                // Only an installed one hides the system's of its name.
                if f.extension().is_some_and(|e| e == "thumbnailer")
                    && !seen.contains(&name)
                    && let Some(t) = std::fs::read_to_string(&f).ok().and_then(|s| parse(&s))
                {
                    seen.insert(name);
                    r.list.push(t);
                }
            }
        }
        r
    }

    /// `globs2` lines `weight:mime:*.ext[:flags]`; only plain `*.suffix` globs.
    fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut f = line.split(':');
            let (Some(w), Some(mime), Some(glob)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let cs = f
                .next()
                .is_some_and(|flags| flags.split(',').any(|x| x == "cs"));
            let Some(suffix) = glob
                .strip_prefix("*.")
                .filter(|s| !s.contains(['*', '?', '[']))
            else {
                continue;
            };
            let (map, key) = match cs {
                true => (&mut self.cs_globs, suffix.to_string()),
                false => (&mut self.globs, suffix.to_lowercase()),
            };
            let w = w.parse().unwrap_or(50);
            let e = map.entry(key).or_insert((0, String::new()));
            if e.1.is_empty() || w > e.0 {
                *e = (w, mime.into());
            }
        }
    }

    pub fn mime_of(&self, name: &str) -> Option<&str> {
        // The longest suffix first: `a.tar.gz` is `tar.gz` before `gz`; a case-sensitive glob
        // (`*.C`) before the others of that suffix.
        (name.match_indices('.'))
            .filter(|&(i, _)| i > 0)
            .find_map(|(i, _)| {
                let suffix = &name[i + 1..];
                (self.cs_globs.get(suffix)).or_else(|| self.globs.get(&suffix.to_lowercase()))
            })
            .map(|(_, m)| m.as_str())
    }

    pub fn for_name(&self, name: &str) -> Option<&Thumbnailer> {
        let mime = self.mime_of(name)?;
        self.list.iter().find(|t| t.mimes.iter().any(|m| m == mime))
    }
}

fn parse(text: &str) -> Option<Thumbnailer> {
    let (mut exec, mut mimes, mut try_exec, mut in_entry) = (None, Vec::new(), None, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Thumbnailer Entry]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| in_entry) else {
            continue;
        };
        match k.trim() {
            "Exec" => exec = Some(v.trim().to_string()),
            "TryExec" => try_exec = Some(v.trim().to_string()),
            "MimeType" => {
                mimes = v
                    .split(';')
                    .filter(|m| !m.is_empty())
                    .map(String::from)
                    .collect()
            }
            _ => {}
        }
    }
    let exec = exec?;
    let prog = try_exec.or_else(|| crate::terminals::exec_words(&exec).into_iter().next())?;
    (installed(&prog) && !mimes.is_empty()).then_some(Thumbnailer { exec, mimes })
}

/// `prog` as a path, or found on `$PATH`.
fn installed(prog: &str) -> bool {
    if prog.contains('/') {
        return Path::new(prog).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

/// `exec`'s arguments for making `out` (a PNG, `size` px) of `input`: `%i` its path, `%u` its URI,
/// `%o` the output, `%s` the size, `%%` a `%`. Paths stay bytes (names not in UTF-8).
pub fn argv(exec: &str, input: &Path, out: &Path, size: u32) -> Vec<OsString> {
    let uri = url::Url::from_file_path(input)
        .map(String::from)
        .unwrap_or_default();
    (crate::terminals::exec_words(exec).iter())
        .map(|w| {
            let mut a = OsString::new();
            let mut chars = w.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    a.push(c.encode_utf8(&mut [0; 4]));
                    continue;
                }
                match chars.next() {
                    Some('i') => a.push(input),
                    Some('u') => a.push(&uri),
                    Some('o') => a.push(out),
                    Some('s') => a.push(size.to_string()),
                    Some('%') | None => a.push("%"),
                    Some(o) => a.push(format!("%{o}")),
                }
            }
            a
        })
        .collect()
}

/// GNOME's layout inside the sandbox (gnome-desktop-thumbnail-script): the file under a fixed name,
/// the output dir as `/tmp` (AppArmor lets evince-thumbnailer write there).
const SANDBOX_IN: &str = "/tmp/gnome-desktop-file-to-thumbnail";
const SANDBOX_OUT: &str = "gnome-desktop-thumbnailer.png";

/// bwrap with the system read-only and nothing else: no home, no network, no other process, no
/// environment but the language. Built once. `--unshare-pid` + `--die-with-parent`: killing bwrap
/// (`run`'s timeout) takes everything inside with it, `--new-session` notwithstanding.
fn bwrap_base() -> &'static [OsString] {
    static BASE: std::sync::OnceLock<Vec<OsString>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        let s = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        let mut a = s(&[
            "bwrap",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/etc",
            "/etc",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--chdir",
            "/",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "GIO_USE_VFS",
            "local",
            // Fonts as the desktop has them, read-only: without the cache each PDF / font
            // rescans them all.
            "--ro-bind-try",
            "/var/cache/fontconfig",
            "/var/cache/fontconfig",
        ]);
        for k in ["LANG", "LC_ALL", "LC_CTYPE"] {
            if let Some(v) = std::env::var_os(k) {
                a.extend(["--setenv".into(), k.into(), v]);
            }
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for d in [".local/share/fonts", ".fonts"] {
                let p = home.join(d);
                a.extend(["--ro-bind-try".into(), p.clone().into(), p.into()]);
            }
        }
        // Merged /usr: links to it; else the dirs themselves.
        for d in ["/lib", "/lib64", "/lib32", "/bin", "/sbin"] {
            match std::fs::read_link(d) {
                Ok(to) => a.extend(["--symlink".into(), to.into(), d.into()]),
                Err(_) if Path::new(d).is_dir() => a.extend(s(&["--ro-bind", d, d])),
                Err(_) => {}
            }
        }
        a
    })
}

/// Whether `exec`'s program is one of the system's (`/usr`, or found there on `$PATH`): only
/// those exist inside the sandbox (not `/opt`, `~/.local/bin`, flatpak exports).
pub fn in_system(exec: &str) -> bool {
    let Some(prog) = crate::terminals::exec_words(exec).into_iter().next() else {
        return false;
    };
    let path = match prog.contains('/') {
        true => Some(PathBuf::from(&prog)),
        false => std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(&prog))
                .find(|p| p.is_file())
        }),
    };
    (path.and_then(|p| p.canonicalize().ok())).is_some_and(|p| p.starts_with("/usr/"))
}

/// The picture a thumbnailer left at `out`, read without trusting it: not through a link, only a
/// regular file, at most `max` bytes (a sandboxed one could leave `/dev/zero`, a FIFO, a link
/// to the home in our dir).
pub fn read_output(out: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(out)
        .ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(max).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// `exec` for `input` inside bwrap, writing into `dir` (empty, ours): the arguments and where the
/// picture comes out.
pub fn sandboxed(exec: &str, input: &Path, dir: &Path, size: u32) -> (Vec<OsString>, PathBuf) {
    let inner = match input.extension() {
        Some(e) => format!("{SANDBOX_IN}.{}", e.to_string_lossy().to_lowercase()),
        None => SANDBOX_IN.into(),
    };
    let out = Path::new("/tmp").join(SANDBOX_OUT);
    let mut a = bwrap_base().to_vec();
    a.extend(["--bind".into(), dir.into(), "/tmp".into()]);
    a.extend(["--ro-bind".into(), input.into(), OsString::from(&inner)]);
    a.extend(argv(exec, Path::new(&inner), &out, size));
    (a, dir.join(SANDBOX_OUT))
}

/// Whether bwrap is there and may make its sandbox (unprivileged user namespaces can be off).
pub fn bwrap_works() -> bool {
    installed("bwrap") && {
        let mut a = bwrap_base().to_vec();
        a.push("true".into());
        run(&a, Duration::from_secs(5))
    }
}

/// Runs `argv` (no input, output dropped) for at most `timeout`; true if it exited fine. Its own
/// process group, all killed on timeout: a worker it started must not write the output later.
pub fn run(argv: &[OsString], timeout: Duration) -> bool {
    use std::process::{Command, Stdio};
    let Some((prog, rest)) = argv.split_first() else {
        return false;
    };
    let mut cmd = Command::new(prog);
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let child = (cmd.args(rest))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                if let Some(g) = rustix::process::Pid::from_raw(child.id() as i32) {
                    let _ = rustix::process::kill_process_group(g, rustix::process::Signal::KILL);
                }
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_thumbnailer_by_the_names_type() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("mime")).unwrap();
        std::fs::create_dir_all(d.join("thumbnailers")).unwrap();
        std::fs::write(
            d.join("mime/globs2"),
            "# comment\n50:application/pdf:*.pdf\n50:application/x-compressed-tar:*.tar.gz\n\
             40:application/gzip:*.gz\n30:text/x-other:*.pdf\n20:x/odd:*.[ch]\n\
             50:text/x-c++src:*.C:cs\n50:text/x-csrc:*.c\n",
        )
        .unwrap();
        let t = |exec: &str, mimes: &str| {
            format!("[Thumbnailer Entry]\nExec={exec}\nMimeType={mimes}\n[Other]\nExec=x\n")
        };
        std::fs::write(
            d.join("thumbnailers/a.thumbnailer"),
            t("sh -s %s %u %o", "application/pdf;"),
        )
        .unwrap();
        // Not installed: left out.
        std::fs::write(
            d.join("thumbnailers/b.thumbnailer"),
            t("no-such-program-here %i %o", "application/gzip"),
        )
        .unwrap();
        let r = Registry::load(&[d.to_path_buf()]);
        assert_eq!(r.mime_of("Doc.PDF"), Some("application/pdf"));
        assert_eq!(r.mime_of("a.tar.gz"), Some("application/x-compressed-tar"));
        assert_eq!(r.mime_of("x.gz"), Some("application/gzip"));
        assert_eq!(
            (r.mime_of(".pdf"), r.mime_of("pdf"), r.mime_of("a.h")),
            (None, None, None)
        );
        assert_eq!(
            r.for_name("a.pdf").map(|t| t.exec.as_str()),
            Some("sh -s %s %u %o")
        );
        assert!(r.for_name("x.gz").is_none());
        assert_eq!(r.mime_of("a.C"), Some("text/x-c++src"));
        assert_eq!(r.mime_of("a.c"), Some("text/x-csrc"));
        // A later dir's heavier glob wins; an override that isn't installed hides nothing.
        let user = tempfile::tempdir().unwrap();
        let u = user.path();
        std::fs::create_dir_all(u.join("mime")).unwrap();
        std::fs::create_dir_all(u.join("thumbnailers")).unwrap();
        std::fs::write(u.join("mime/globs2"), "10:x/foo:*.pdf\n").unwrap();
        std::fs::write(
            u.join("thumbnailers/a.thumbnailer"),
            t("gone-program %i", "application/pdf"),
        )
        .unwrap();
        let r = Registry::load(&[u.to_path_buf(), d.to_path_buf()]);
        assert_eq!(r.mime_of("a.pdf"), Some("application/pdf"));
        assert_eq!(
            r.for_name("a.pdf").map(|t| t.exec.as_str()),
            Some("sh -s %s %u %o")
        );
    }

    #[test]
    fn sandboxed_sees_only_its_file() {
        if !bwrap_works() {
            eprintln!("no working bwrap here: skipped");
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let input = home.path().join("Doc.PDF");
        std::fs::write(&input, "pdf").unwrap();
        let out = tempfile::tempdir().unwrap();
        let (a, png) = sandboxed("cp %i %o", &input, out.path(), 128);
        assert!(run(&a, Duration::from_secs(10)));
        assert_eq!(std::fs::read_to_string(&png).unwrap(), "pdf");
        // The file's own dir is not there inside.
        let probe = format!("test -e {}", input.display());
        let (a, _) = sandboxed(&probe, &input, out.path(), 128);
        assert!(!run(&a, Duration::from_secs(10)));
        // Nor the home, nor the environment.
        let home = std::env::var("HOME").unwrap();
        let (a, _) = sandboxed(
            &format!("test -e {home}/.bashrc -o -e {home}/.config"),
            &input,
            out.path(),
            128,
        );
        assert!(!run(&a, Duration::from_secs(10)));
        let (a, png) = sandboxed("sh -c \"env > %o\"", &input, out.path(), 128);
        assert!(run(&a, Duration::from_secs(10)));
        let env = std::fs::read_to_string(&png).unwrap();
        assert!(
            !env.contains("DBUS_SESSION_BUS_ADDRESS") && env.contains("HOME=/tmp"),
            "{env}"
        );
    }

    #[test]
    fn the_output_is_read_without_trusting_it() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("secret"), "key").unwrap();
        std::fs::write(d.join("ok.png"), "png").unwrap();
        symlink(d.join("secret"), d.join("link.png")).unwrap();
        symlink("/dev/zero", d.join("zero.png")).unwrap();
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            d.join("fifo.png"),
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        assert_eq!(
            read_output(&d.join("ok.png"), 10).as_deref(),
            Some(b"png".as_slice())
        );
        assert_eq!(read_output(&d.join("ok.png"), 2), None, "too big");
        for bad in ["link.png", "zero.png", "fifo.png", "none.png"] {
            assert_eq!(read_output(&d.join(bad), 10), None, "{bad}");
        }
        assert!(
            in_system("sh -c x") && !in_system("/opt/x/thumb %i") && !in_system("no-such-prog")
        );
    }

    #[test]
    fn run_kills_the_whole_group_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let late = dir.path().join("late");
        let script = format!("(sleep 1; touch '{}') & sleep 5", late.display());
        let argv: Vec<OsString> = ["sh", "-c", &script].map(OsString::from).to_vec();
        assert!(!run(&argv, Duration::from_millis(200)));
        std::thread::sleep(Duration::from_millis(1300));
        assert!(!late.exists(), "a worker outlived the timeout");
        assert!(run(&["true".into()], Duration::from_secs(5)));
        assert!(!run(&["false".into()], Duration::from_secs(5)));
    }

    #[test]
    fn argv_fills_the_codes_and_keeps_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let input = Path::new(std::ffi::OsStr::from_bytes(b"/d/caf\xe9 1.pdf"));
        let a = argv(
            "\"my thumb\" -s %s --in=%i %u %o 100%%",
            input,
            Path::new("/t/o.png"),
            128,
        );
        let want: Vec<OsString> = vec![
            "my thumb".into(),
            "-s".into(),
            "128".into(),
            OsString::from_vec([b"--in=".as_slice(), input.as_os_str().as_bytes()].concat()),
            "file:///d/caf%E9%201.pdf".into(),
            "/t/o.png".into(),
            "100%".into(),
        ];
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(a, want);
    }
}
