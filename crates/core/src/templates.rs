//! Files → «New file from template» (TC `cm_EditFileMenu`): the files in the XDG templates dir
//! (`~/Шаблоны`, `~/Templates`: `XDG_TEMPLATES_DIR` in `user-dirs.dirs`), copied to a new name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The templates dir: `XDG_TEMPLATES_DIR` in `<config>/user-dirs.dirs` (`$HOME/…`, `${HOME}/…`
/// or absolute), else `~/Templates`. `config`: `$XDG_CONFIG_HOME` (`None`: `~/.config`).
pub fn dir(home: &Path, config: Option<PathBuf>) -> PathBuf {
    let config = config.unwrap_or_else(|| home.join(".config"));
    let text = fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let set = (text.lines())
        .filter_map(|l| l.trim().strip_prefix("XDG_TEMPLATES_DIR="))
        .map(|v| v.trim().trim_matches('"'))
        .next_back();
    let in_home = |v: &str| {
        let rest = (v.strip_prefix("${HOME}")).or_else(|| v.strip_prefix("$HOME"))?;
        (rest.is_empty() || rest.starts_with('/')).then(|| home.join(rest.trim_start_matches('/')))
    };
    match set {
        Some(v) if v.starts_with('/') => PathBuf::from(v),
        Some(v) => in_home(v).unwrap_or_else(|| home.join("Templates")),
        None => home.join("Templates"),
    }
}

/// The templates: the dir's files (not hidden), by name. A dir that is the home dir itself
/// (`XDG_TEMPLATES_DIR="$HOME/"`, as `xdg-user-dirs` writes for «none») has none.
pub fn list(dir: &Path, home: &Path) -> Vec<PathBuf> {
    if dir == home {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = (fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| e.path().is_file())
        .map(|e| e.path())
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files
}

/// A copy of `template` as `dest` with its permission bits (a script stays runnable), never over
/// a file there (its own error then).
// ponytail: written in place, on the caller's thread: templates are a few KB on the local disk;
// a big one or a slow mount wants `ops::transfer`'s part file + rename, off the UI thread.
pub fn create(template: &Path, dest: &Path) -> io::Result<()> {
    let mut to = fs::File::create_new(dest)?;
    let copied = fs::File::open(template)
        .and_then(|mut from| io::copy(&mut from, &mut to))
        .and_then(|_| to.set_permissions(fs::metadata(template)?.permissions()));
    if let Err(e) = copied {
        drop(to);
        let _ = fs::remove_file(dest); // not a half file left behind
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dir_comes_from_user_dirs_and_files_are_copied_once() {
        let home = tempfile::tempdir().unwrap();
        let (h, cfg) = (home.path(), home.path().join(".config"));
        fs::create_dir_all(&cfg).unwrap();
        let cfg_in = || Some(cfg.clone());
        assert_eq!(dir(h, cfg_in()), h.join("Templates")); // no user-dirs.dirs
        let dirs = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Рабочий стол\"\nXDG_TEMPLATES_DIR=\"$HOME/Шаблоны\"\n";
        fs::write(cfg.join("user-dirs.dirs"), dirs).unwrap();
        let t = dir(h, cfg_in());
        assert_eq!(t, h.join("Шаблоны"));
        assert_eq!(dir(h, None), t); // no $XDG_CONFIG_HOME: ~/.config, the same file
        for (line, want) in [
            ("${HOME}/T", h.join("T")),
            ("$HOMEDIR/T", h.join("Templates")),
        ] {
            fs::write(
                cfg.join("user-dirs.dirs"),
                format!("XDG_TEMPLATES_DIR=\"{line}\"\n"),
            )
            .unwrap();
            assert_eq!(dir(h, cfg_in()), want);
        }
        // «None»: the home dir itself, no templates.
        fs::write(cfg.join("user-dirs.dirs"), "XDG_TEMPLATES_DIR=\"$HOME/\"\n").unwrap();
        assert!(list(&dir(h, cfg_in()), h).is_empty());
        fs::create_dir(&t).unwrap();
        for (n, text) in [("Текст.txt", "hi"), ("a.odt", "zip"), (".hidden", "")] {
            fs::write(t.join(n), text).unwrap();
        }
        fs::create_dir(t.join("sub")).unwrap();
        let names: Vec<_> = list(&t, h)
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.odt", "Текст.txt"]);
        let out = h.join("new.txt");
        create(&t.join("Текст.txt"), &out).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "hi");
        // Its permission bits too.
        use std::os::unix::fs::PermissionsExt;
        let script = t.join("run.sh");
        fs::write(&script, "#!/bin/sh").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        create(&script, &h.join("run.sh")).unwrap();
        assert_eq!(
            fs::metadata(h.join("run.sh")).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::write(&out, "mine").unwrap();
        assert!(create(&t.join("Текст.txt"), &out).is_err()); // there: kept
        assert_eq!(fs::read_to_string(&out).unwrap(), "mine");
    }
}
