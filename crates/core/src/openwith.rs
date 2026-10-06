//! "Open with": the programs for a file's type, through the `gio` CLI (no glib in the build).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
    /// `Icon=`: a theme icon name or an absolute path; empty if none.
    pub icon: String,
    /// The `.desktop` file, as `gio launch` wants it.
    pub desktop: PathBuf,
}

/// `standard::content-type` from `gio info -a standard::content-type`.
pub fn parse_content_type(out: &str) -> Option<&str> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("standard::content-type:"))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Desktop ids from `gio mime TYPE`: the default first, then the others, no repeats.
pub fn parse_mime(out: &str) -> Vec<String> {
    let mut lines = out.lines();
    // "Default application for “x”: id.desktop" or "No default applications for “x”".
    let default = lines
        .next()
        .and_then(|l| l.rsplit_once(": "))
        .map(|(_, id)| id.trim());
    let mut ids: Vec<String> = default.into_iter().map(String::from).collect();
    for l in lines.filter_map(|l| l.strip_prefix('\t')) {
        let id = l.trim();
        if !id.is_empty() && !ids.iter().any(|x| x == id) {
            ids.push(id.into());
        }
    }
    ids
}

/// The programs for a type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub mime: String,
    pub apps: Vec<App>,
}

/// The name a `.desktop` file shows in `lang` ("ru"), its icon and whether menus show it
/// (`NoDisplay`: still a handler for its types); `None` if `Hidden` (deleted) or not a program
/// (`Type=Link` / `Directory`).
pub fn parse_desktop(text: &str, lang: &str) -> Option<(String, String, bool)> {
    let (mut plain, mut local, mut in_entry) = (None, None, false);
    let (mut icon, mut shown) = (String::new(), true);
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        let Some((k, v)) = l.split_once('=').filter(|_| in_entry) else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k.strip_prefix("Name[").and_then(|k| k.strip_suffix(']')) {
            // Name[ru] or Name[ru_RU]; the exact language wins.
            Some(l) if l == lang || (local.is_none() && l.split('_').next() == Some(lang)) => {
                local = Some(v.to_string());
            }
            Some(_) => {}
            None if k == "Name" => plain = Some(v.to_string()),
            None if k == "Hidden" && v == "true" => return None,
            None if k == "NoDisplay" => shown = v != "true",
            None if k == "Type" && v != "Application" => return None,
            None if k == "Icon" => icon = v.to_string(),
            None => {}
        }
    }
    Some((local.or(plain)?, icon, shown))
}

/// Where `.desktop` files live: `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`.
pub fn data_dirs() -> Vec<PathBuf> {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty());
    let home = env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| Path::new(&h).join(".local/share")));
    let dirs = env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&dirs))
        .collect()
}

// ponytail: no applications/ subdirs (id "kde-foo" → kde/foo.desktop); add if such apps go missing.
pub fn find_desktop(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join("applications").join(id))
        .find(|p| p.is_file())
}

fn gio(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = Command::new("gio")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The programs for `file`'s type, the default first. Blocking: runs `gio` twice.
pub fn apps(file: &Path, lang: &str) -> Result<Found, String> {
    let info = gio(&[
        "info".as_ref(),
        "-a".as_ref(),
        "standard::content-type".as_ref(),
        file.as_ref(),
    ])?;
    let mime = parse_content_type(&info).ok_or("no content type")?;
    let dirs = data_dirs();
    let apps = parse_mime(&gio(&["mime".as_ref(), mime.as_ref()])?)
        .iter()
        .filter_map(|id| {
            let desktop = find_desktop(id, &dirs)?;
            let (name, icon, _) = parse_desktop(&std::fs::read_to_string(&desktop).ok()?, lang)?;
            Some(App {
                name,
                icon,
                desktop,
            })
        })
        .collect();
    Ok(Found {
        mime: mime.into(),
        apps,
    })
}

/// Every program in `dirs` (`data_dirs()`), by name; an id in an earlier dir hides later ones.
pub fn all_apps(dirs: &[PathBuf], lang: &str) -> Vec<App> {
    let mut seen = std::collections::HashSet::new();
    let mut apps: Vec<App> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d.join("applications")).ok())
        .flat_map(|rd| rd.filter_map(Result::ok).map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
        .filter(|p| seen.insert(p.file_name().map(ToOwned::to_owned)))
        .filter_map(|desktop| {
            let text = std::fs::read_to_string(&desktop).ok()?;
            let (name, icon, shown) = parse_desktop(&text, lang)?;
            shown.then_some(())?;
            Some(App {
                name,
                icon,
                desktop,
            })
        })
        .collect();
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    apps
}

/// Makes `desktop` the default program for `mime`. Blocking.
pub fn set_default(mime: &str, desktop: &Path) -> Result<(), String> {
    let id = desktop.file_name().ok_or("no desktop file")?;
    gio(&["mime".as_ref(), mime.as_ref(), id]).map(drop)
}

/// "Other program…": the typed command with the files after it, for `sh -c`.
pub fn other_line(cmd: &str, files: &[PathBuf]) -> String {
    files.iter().fold(cmd.trim().to_string(), |line, f| {
        crate::cmdline::append(&line, &crate::cmdline::quote(&f.display().to_string()))
    })
}

pub fn launch_argv(desktop: &Path, files: &[PathBuf]) -> Vec<OsString> {
    let mut argv = vec![OsString::from("gio"), "launch".into(), desktop.into()];
    argv.extend(files.iter().map(OsString::from));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_from_gio_info() {
        let out = "uri: file:///a.md\nattributes:\n  standard::content-type: text/markdown\n";
        assert_eq!(parse_content_type(out), Some("text/markdown"));
        assert_eq!(parse_content_type("attributes:\n"), None);
    }

    #[test]
    fn mime_default_first_without_repeats() {
        let out = "Default application for “text/markdown”: b.desktop\n\
                   Registered applications:\n\ta.desktop\n\tb.desktop\n\
                   Recommended applications:\n\ta.desktop\n\tc.desktop\n";
        assert_eq!(parse_mime(out), ["b.desktop", "a.desktop", "c.desktop"]);
        assert!(parse_mime("No default applications for “x/y”\n").is_empty());
    }

    #[test]
    fn desktop_name_in_language() {
        let t = "[Desktop Entry]\nName=Editor\nName[ru_RU]=Ред\nName[de]=Ed\nExec=e %f\n\
                 [Desktop Action new]\nName[ru]=Новое окно\n";
        let name = |t, l| parse_desktop(t, l).map(|(n, ..)| n);
        assert_eq!(name(t, "ru").as_deref(), Some("Ред"));
        assert_eq!(name(t, "fr").as_deref(), Some("Editor"));
        let exact = "[Desktop Entry]\nName[ru_RU]=Долго\nName[ru]=Ред\nName=E\n";
        assert_eq!(name(exact, "ru").as_deref(), Some("Ред"));
        for hidden in ["Hidden=true", "Type=Link"] {
            let t = format!("[Desktop Entry]\nName=A\n{hidden}\n");
            assert_eq!(parse_desktop(&t, "ru"), None, "{hidden}");
        }
        assert!(parse_desktop("[Desktop Entry]\nType=Application\nName=A\n", "ru").is_some());
        // Not in menus, still a handler for its types.
        let nd = parse_desktop("[Desktop Entry]\nName=A\nNoDisplay=true\n", "ru").unwrap();
        assert!(!nd.2);
    }

    #[test]
    fn desktop_icon_name_or_path() {
        let t = "[Desktop Entry]\nName=A\nIcon=org.gnome.gedit\n[Desktop Action x]\nIcon=other\n";
        assert_eq!(parse_desktop(t, "ru").unwrap().1, "org.gnome.gedit");
        let none = parse_desktop("[Desktop Entry]\nName=A\n", "ru").unwrap();
        assert_eq!(none.1, "");
    }

    #[test]
    fn other_program_gets_quoted_files() {
        let files = ["/a b/x.txt".into(), "/c".into()];
        assert_eq!(other_line(" gimp ", &files), "gimp '/a b/x.txt' /c");
        assert_eq!(other_line("foot -e less", &[]), "foot -e less");
    }

    #[test]
    fn desktop_found_in_first_dir_that_has_it() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir(b.path().join("applications")).unwrap();
        let f = b.path().join("applications/x.desktop");
        std::fs::write(&f, "").unwrap();
        let dirs = [a.path().to_path_buf(), b.path().to_path_buf()];
        assert_eq!(find_desktop("x.desktop", &dirs), Some(f));
        assert_eq!(find_desktop("y.desktop", &dirs), None);
    }

    #[test]
    fn all_apps_by_name_first_dir_wins() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let put = |d: &Path, id: &str, text: &str| {
            std::fs::create_dir_all(d.join("applications")).unwrap();
            std::fs::write(d.join("applications").join(id), text).unwrap();
        };
        put(a.path(), "x.desktop", "[Desktop Entry]\nName=zeta\n");
        put(b.path(), "x.desktop", "[Desktop Entry]\nName=shadowed\n");
        put(
            b.path(),
            "y.desktop",
            "[Desktop Entry]\nName=Alpha\nIcon=y\n",
        );
        put(
            b.path(),
            "h.desktop",
            "[Desktop Entry]\nName=hid\nNoDisplay=true\n",
        );
        put(b.path(), "n.txt", "[Desktop Entry]\nName=not\n");
        let dirs = [a.path().to_path_buf(), b.path().to_path_buf()];
        let names: Vec<String> = all_apps(&dirs, "ru").into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Alpha", "zeta"]);
    }

    #[test]
    fn launch_passes_all_files() {
        let argv = launch_argv(Path::new("/a/x.desktop"), &["/f 1".into(), "/g".into()]);
        assert_eq!(
            argv,
            ["gio", "launch", "/a/x.desktop", "/f 1", "/g"].map(OsString::from)
        );
    }
}
