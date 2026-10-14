//! About → «Check for updates»: the latest release tag from GitHub, compared with ours.

use std::process::Command;

pub const LATEST_API: &str =
    "https://api.github.com/repos/shagovAlexei/cosmic-shagoff-commander/releases/latest";

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Checking,
    Latest,
    /// The newer release's tag (`v0.18.0`).
    Available(String),
    Failed(String),
}

/// A string field (`"tag_name": "v0.17.0"`) out of the release JSON (no JSON parser for two fields).
pub fn field<'a>(json: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\"");
    let rest = &json[json.find(&key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// `tag` (`v0.18.0`) is a later version than `current` (`0.17.0`); anything but N.N.N → false.
pub fn newer(current: &str, tag: &str) -> bool {
    let nums = |s: &str| -> Option<Vec<u64>> {
        let v: Vec<u64> = s
            .trim_start_matches('v')
            .split('.')
            .map(|n| n.parse().ok())
            .collect::<Option<_>>()?;
        (v.len() == 3).then_some(v)
    };
    matches!((nums(current), nums(tag)), (Some(c), Some(t)) if t > c)
}

/// What GitHub's answer (`json`) means for `current`.
pub fn status(current: &str, json: &str) -> Status {
    match field(json, "tag_name") {
        Some(t) if newer(current, t) => Status::Available(t.to_string()),
        Some(_) => Status::Latest,
        // An API error (rate limit…) says why in "message".
        None => Status::Failed(field(json, "message").unwrap_or("no tag_name").to_string()),
    }
}

/// Ask GitHub through `curl` (10 s limit). Blocking.
pub fn check(current: &str) -> Status {
    let out = match Command::new("curl")
        .args(["-sSL", "-m", "10", LATEST_API])
        .output()
    {
        Ok(o) => o,
        Err(e) => return Status::Failed(format!("curl: {e}")),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Status::Failed(if err.is_empty() {
            format!("curl: {}", out.status)
        } else {
            err
        });
    }
    status(current, &String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_release_json() {
        let j = r#"{"url":"x","tag_name" : "v0.18.0","name":"v0.18.0"}"#;
        assert_eq!(status("0.17.0", j), Status::Available("v0.18.0".into()));
        assert_eq!(status("0.18.0", j), Status::Latest);
        let limit = r#"{"message":"API rate limit exceeded for 1.2.3.4.","documentation_url":"x"}"#;
        assert_eq!(
            status("0.18.0", limit),
            Status::Failed("API rate limit exceeded for 1.2.3.4.".into())
        );
    }

    #[test]
    fn newer_compares_numbers() {
        assert!(newer("0.17.0", "v0.18.0"));
        assert!(newer("0.9.0", "v0.10.0")); // not by text
        assert!(!newer("0.17.0", "v0.17.0"));
        assert!(!newer("0.17.1", "v0.17.0"));
        assert!(!newer("0.17.0", "nightly"));
        assert!(!newer("0.18.0", "v0.18.0.0"));
    }
}
