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

/// The check at start runs at most once a day.
const DAY: u64 = 86_400;

/// A check is due: `last` (unix seconds, 0 = never) is a day or more before `now`; a clock set
/// back counts as due too.
pub fn due(last: u64, now: u64) -> bool {
    now < last || now - last >= DAY
}

/// The `.deb` a release carries, by its tag.
pub fn deb_name(tag: &str) -> String {
    format!(
        "shagoff-commander_{}_amd64.deb",
        tag.trim_start_matches('v')
    )
}

/// Updating in place goes through apt on an x86-64 system; elsewhere the release page opens.
pub fn can_install() -> bool {
    cfg!(target_arch = "x86_64")
        && ["apt-get", "pkexec"].iter().all(|p| {
            std::env::var_os("PATH")
                .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
        })
}

/// `name`'s hash in the text of a `SHA256SUMS`.
pub fn expected_sum(sums: &[u8], name: &str) -> Option<String> {
    crate::checksum::parse(sums)
        .into_iter()
        .find(|(_, n)| n.as_os_str() == name)
        .map(|(h, _)| h)
}

fn fetch(url: &str, to: &std::path::Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "-m", "600", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// The release `tag`'s `.deb` into `dir`, its SHA-256 checked against the release's `SHA256SUMS`
/// (a release without one, or a file that does not match, is refused). Blocking.
pub fn download(tag: &str, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let base =
        format!("https://github.com/shagovAlexei/cosmic-shagoff-commander/releases/download/{tag}");
    let name = deb_name(tag);
    let (deb, sums) = (dir.join(&name), dir.join("SHA256SUMS"));
    fetch(&format!("{base}/{name}"), &deb)?;
    fetch(&format!("{base}/SHA256SUMS"), &sums)?;
    let want = expected_sum(&std::fs::read(&sums).map_err(|e| e.to_string())?, &name)
        .ok_or_else(|| format!("{name} is not in SHA256SUMS"))?;
    let stop = std::sync::atomic::AtomicBool::new(false);
    let got = crate::checksum::hash(&deb, crate::checksum::Algo::Sha256, &stop)
        .map_err(|e| e.to_string())?;
    match got == want {
        true => Ok(deb),
        false => Err(format!("{name}: SHA-256 does not match")),
    }
}

/// `pkexec apt-get install` of the downloaded `.deb` (asks for the password itself). Blocking.
pub fn install(deb: &std::path::Path) -> Result<(), String> {
    let out = Command::new("pkexec")
        .args(["apt-get", "install", "-y"])
        .arg(deb)
        .output()
        .map_err(|e| format!("pkexec: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.lines().last().unwrap_or("apt-get failed").to_string())
        }
    }
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

    #[test]
    fn due_once_a_day() {
        assert!(due(0, 1_000_000));
        assert!(!due(1_000_000, 1_000_000 + 86_399));
        assert!(due(1_000_000, 1_000_000 + 86_400));
        assert!(due(2_000_000, 1_000_000)); // the clock went back
    }

    #[test]
    fn the_deb_and_its_sum() {
        assert_eq!(deb_name("v0.118.0"), "shagoff-commander_0.118.0_amd64.deb");
        let sums = format!(
            "{}  shagoff-commander_0.118.0_amd64.deb\n{}  x.rpm\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert_eq!(
            expected_sum(sums.as_bytes(), "shagoff-commander_0.118.0_amd64.deb"),
            Some("a".repeat(64))
        );
        assert_eq!(expected_sum(sums.as_bytes(), "other.deb"), None);
    }
}
