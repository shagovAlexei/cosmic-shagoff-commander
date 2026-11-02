//! A crash's report (214c): written by the panic hook, offered at the next start as a GitHub issue
//! the user reads and sends themselves (nothing leaves the machine on its own).

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// `$XDG_STATE_HOME/shagoff-commander/crash.log` (`~/.local/state/…`).
pub fn log_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|h| h.join(".local/state")))?;
    Some(state.join("shagoff-commander/crash.log"))
}

/// `report` into `log` (the last crash only, 0600). From the panic hook: errors are dropped.
pub fn write(log: &Path, report: &str) {
    let _ = log.parent().map(fs::create_dir_all);
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(log);
    let _ = f.and_then(|mut f| f.write_all(report.as_bytes()));
}

/// The report a crash left, moved aside (`crash.log.old`): offered once.
pub fn take(log: &Path) -> Option<String> {
    let text = fs::read_to_string(log).ok()?;
    let _ = fs::rename(log, log.with_extension("log.old"));
    (!text.trim().is_empty()).then_some(text)
}

/// A new issue in `repo` with `report` (cut to its first 4000 bytes: an address has a limit).
pub fn issue_url(repo: &str, report: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let mut end = report.len().min(4000);
    while !report.is_char_boundary(end) {
        end -= 1;
    }
    let first = report.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let title = format!("Crash: {}", first.chars().take(100).collect::<String>());
    let body = format!("```\n{}\n```\n", &report[..end]);
    let enc = |s: &str| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string();
    format!(
        "{repo}/issues/new?title={}&body={}",
        enc(&title),
        enc(&body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_is_offered_once_as_an_issue() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("s/crash.log");
        assert_eq!(take(&log), None);
        write(&log, "old");
        write(&log, "panicked at src/x.rs:1:2:\nboom & more");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let r = take(&log).unwrap();
        assert_eq!(r, "panicked at src/x.rs:1:2:\nboom & more"); // the last one only
        assert_eq!(take(&log), None);
        let url = issue_url("https://github.com/o/r", &r);
        assert!(url.starts_with("https://github.com/o/r/issues/new?title=Crash%3A%20panicked"));
        assert!(url.contains("boom%20%26%20more") && !url.contains(' '));
        assert!(issue_url("r", &"я".repeat(5000)).len() < 40000); // cut on a char boundary
    }
}
