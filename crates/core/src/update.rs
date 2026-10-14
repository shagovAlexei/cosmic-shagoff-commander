//! About → «Check for updates»: the latest release tag from GitHub, compared with ours.

use std::process::Command;

pub const LATEST_API: &str =
    "https://api.github.com/repos/shagovAlexei/cosmic-shagoff-commander/releases/latest";

/// `"tag_name": "v0.17.0"` out of the release JSON (no JSON parser for one field).
pub fn tag(json: &str) -> Option<&str> {
    let rest = &json[json.find("\"tag_name\"")? + 10..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// `tag` (`v0.18.0`) is a later version than `current` (`0.17.0`); unparsable → false.
pub fn newer(current: &str, tag: &str) -> bool {
    let nums = |s: &str| -> Option<Vec<u64>> {
        s.trim_start_matches('v')
            .split('.')
            .map(|n| n.parse().ok())
            .collect()
    };
    matches!((nums(current), nums(tag)), (Some(c), Some(t)) if t > c)
}

/// The latest release tag, through `curl` (10 s limit). Blocking.
pub fn latest() -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-sSfL", "-m", "10", LATEST_API])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let json = String::from_utf8_lossy(&out.stdout);
    tag(&json)
        .map(str::to_string)
        .ok_or_else(|| "no tag_name".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_from_release_json() {
        let j = r#"{"url":"x","tag_name" : "v0.17.0","name":"v0.17.0"}"#;
        assert_eq!(tag(j), Some("v0.17.0"));
        assert_eq!(tag(r#"{"message":"Not Found"}"#), None);
    }

    #[test]
    fn newer_compares_numbers() {
        assert!(newer("0.17.0", "v0.18.0"));
        assert!(newer("0.9.0", "v0.10.0")); // not by text
        assert!(!newer("0.17.0", "v0.17.0"));
        assert!(!newer("0.17.1", "v0.17.0"));
        assert!(!newer("0.17.0", "nightly"));
    }
}
