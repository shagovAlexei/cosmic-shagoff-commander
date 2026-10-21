//! The desktop's thumbnailers (freedesktop: `thumbnailers/*.thumbnailer` in the data dirs) for
//! the Thumbnails view's files we can't decode ourselves: PDF, video, SVG, fonts…
//! The file's type comes from its name, by shared-mime-info's `mime/globs2`.
// ponytail: by name only (no magic sniffing, no mime aliases / subclasses); add if types go missing.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub struct Thumbnailer {
    pub exec: String,
    pub mimes: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Registry {
    list: Vec<Thumbnailer>,
    /// Lowercase name suffix (`pdf`, `tar.gz`) → mime type, the heaviest glob kept.
    globs: HashMap<String, String>,
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
                if f.extension().is_some_and(|e| e == "thumbnailer")
                    && seen.insert(name)
                    && let Some(t) = std::fs::read_to_string(&f).ok().and_then(|s| parse(&s))
                {
                    r.list.push(t);
                }
            }
        }
        r
    }

    /// `globs2` lines `weight:mime:*.ext[:flags]`, heaviest first; only plain `*.suffix` globs.
    fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut f = line.split(':');
            let (Some(_), Some(mime), Some(glob)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            if let Some(suffix) = glob.strip_prefix("*.")
                && !suffix.contains(['*', '?', '['])
            {
                (self.globs)
                    .entry(suffix.to_lowercase())
                    .or_insert_with(|| mime.into());
            }
        }
    }

    pub fn mime_of(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        // The longest suffix first: `a.tar.gz` is `tar.gz` before `gz`.
        (lower.match_indices('.'))
            .filter(|&(i, _)| i > 0)
            .find_map(|(i, _)| self.globs.get(&lower[i + 1..]))
            .map(String::as_str)
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
             40:application/gzip:*.gz\n30:text/x-other:*.pdf\n20:x/odd:*.[ch]\n",
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
            (r.mime_of(".pdf"), r.mime_of("pdf"), r.mime_of("a.c")),
            (None, None, None)
        );
        assert_eq!(
            r.for_name("a.pdf").map(|t| t.exec.as_str()),
            Some("sh -s %s %u %o")
        );
        assert!(r.for_name("x.gz").is_none());
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
