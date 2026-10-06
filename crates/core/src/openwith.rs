//! "Open with": the programs for a file's type, through the `gio` CLI (no glib in the build).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
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

/// The name a `.desktop` file shows in `lang` ("ru"), or `None` if it hides itself.
pub fn parse_desktop(text: &str, lang: &str) -> Option<String> {
    let (mut plain, mut local, mut in_entry) = (None, None, false);
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
            None => {}
        }
    }
    local.or(plain)
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
pub fn apps(file: &Path, lang: &str) -> Result<Vec<App>, String> {
    let info = gio(&[
        "info".as_ref(),
        "-a".as_ref(),
        "standard::content-type".as_ref(),
        file.as_ref(),
    ])?;
    let mime = parse_content_type(&info).ok_or("no content type")?;
    let dirs = data_dirs();
    Ok(parse_mime(&gio(&["mime".as_ref(), mime.as_ref()])?)
        .iter()
        .filter_map(|id| {
            let desktop = find_desktop(id, &dirs)?;
            let name = parse_desktop(&std::fs::read_to_string(&desktop).ok()?, lang)?;
            Some(App { name, desktop })
        })
        .collect())
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
        assert_eq!(parse_desktop(t, "ru").as_deref(), Some("Ред"));
        assert_eq!(parse_desktop(t, "fr").as_deref(), Some("Editor"));
        let exact = "[Desktop Entry]\nName[ru_RU]=Долго\nName[ru]=Ред\nName=E\n";
        assert_eq!(parse_desktop(exact, "ru").as_deref(), Some("Ред"));
        assert_eq!(
            parse_desktop("[Desktop Entry]\nName=A\nHidden=true\n", "ru"),
            None
        );
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
    fn launch_passes_all_files() {
        let argv = launch_argv(Path::new("/a/x.desktop"), &["/f 1".into(), "/g".into()]);
        assert_eq!(
            argv,
            ["gio", "launch", "/a/x.desktop", "/f 1", "/g"].map(OsString::from)
        );
    }
}
