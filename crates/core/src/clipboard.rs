//! System clipboard payloads for files, in cosmic-files' format
//! (pop-os/cosmic-files `src/clipboard.rs`, GPL-3.0-only).

use std::path::PathBuf;
use url::Url;

pub const GNOME: &str = "x-special/gnome-copied-files";
pub const URI_LIST: &str = "text/uri-list";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Cut,
}

/// The three payloads one copy/cut offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mime {
    /// `copy|cut` then one `file://` URI per line.
    pub gnome: String,
    /// Each URI followed by `\r\n`.
    pub uri_list: String,
    /// Paths joined by `\r\n`; non-UTF-8 paths are left out.
    pub plain: String,
}

pub fn encode(kind: Kind, paths: &[PathBuf]) -> Mime {
    let mut m = Mime {
        gnome: match kind {
            Kind::Copy => "copy",
            Kind::Cut => "cut",
        }
        .into(),
        uri_list: String::new(),
        plain: String::new(),
    };
    for p in paths {
        if let Some(s) = p.to_str() {
            if !m.plain.is_empty() {
                m.plain.push_str("\r\n");
            }
            m.plain.push_str(s);
        }
        if let Ok(url) = Url::from_file_path(p) {
            m.uri_list.push_str(url.as_str());
            m.uri_list.push_str("\r\n");
            m.gnome.push('\n');
            m.gnome.push_str(url.as_str());
        }
    }
    m
}

/// `None` for an unknown mime type, a non-`file://` URI or a malformed header.
pub fn decode(mime: &str, data: &[u8]) -> Option<(Kind, Vec<PathBuf>)> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let kind = match mime {
        GNOME => match lines.next()? {
            "copy" => Kind::Copy,
            "cut" => Kind::Cut,
            _ => return None,
        },
        URI_LIST => Kind::Copy,
        _ => return None,
    };
    let paths = lines
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| Url::parse(l).ok()?.to_file_path().ok())
        .collect::<Option<Vec<_>>>()?;
    Some((kind, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<PathBuf> {
        vec!["/tmp/a b".into(), "/tmp/Отчёт 100%#1.txt".into()]
    }

    #[test]
    fn gnome_round_trip_keeps_kind_and_odd_names() {
        for kind in [Kind::Copy, Kind::Cut] {
            let m = encode(kind, &paths());
            assert_eq!(decode(GNOME, m.gnome.as_bytes()), Some((kind, paths())));
        }
    }

    #[test]
    fn gnome_layout_matches_cosmic_files() {
        let m = encode(Kind::Cut, &["/tmp/a b".into()]);
        assert_eq!(m.gnome, "cut\nfile:///tmp/a%20b");
        assert_eq!(m.uri_list, "file:///tmp/a%20b\r\n");
        assert_eq!(encode(Kind::Copy, &["/x".into()]).gnome, "copy\nfile:///x");
    }

    #[test]
    fn uri_list_round_trip_is_copy() {
        let m = encode(Kind::Cut, &paths());
        assert_eq!(
            decode(URI_LIST, m.uri_list.as_bytes()),
            Some((Kind::Copy, paths()))
        );
    }

    #[test]
    fn plain_is_paths_joined_by_crlf() {
        assert_eq!(
            encode(Kind::Copy, &paths()).plain,
            "/tmp/a b\r\n/tmp/Отчёт 100%#1.txt"
        );
    }

    #[test]
    fn uri_list_skips_comments_and_blank_lines() {
        let data = "# from some app\r\nfile:///tmp/x\r\n\r\n";
        assert_eq!(
            decode(URI_LIST, data.as_bytes()),
            Some((Kind::Copy, vec!["/tmp/x".into()]))
        );
    }

    #[test]
    fn gnome_accepts_crlf_and_trailing_newline() {
        let data = "copy\r\nfile:///tmp/x\r\n";
        assert_eq!(
            decode(GNOME, data.as_bytes()),
            Some((Kind::Copy, vec!["/tmp/x".into()]))
        );
    }

    #[test]
    fn rejects_non_file_uri_bad_header_and_unknown_mime() {
        assert_eq!(decode(URI_LIST, b"https://example.com/a\r\n"), None);
        assert_eq!(decode(GNOME, b"move\nfile:///tmp/x"), None);
        assert_eq!(decode(GNOME, b""), None);
        assert_eq!(decode("text/plain", b"/tmp/x"), None);
        assert_eq!(decode(URI_LIST, b"not a uri"), None);
    }
}
