//! About → «Check for updates»: the latest release tag from GitHub, compared with ours.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "https://github.com/shagovAlexei/cosmic-shagoff-commander";
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

/// Updating in place goes through apt on an x86-64 system; elsewhere the release page opens.
pub fn can_install() -> bool {
    cfg!(target_arch = "x86_64")
        && ["apt-get", "pkexec"].iter().all(|p| {
            std::env::var_os("PATH")
                .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
        })
}

/// `v0.18.0`: the only shape of a tag we put in a download address.
pub fn valid_tag(tag: &str) -> bool {
    tag.strip_prefix('v').is_some_and(|v| {
        let parts: Vec<&str> = v.split('.').collect();
        parts.len() == 3
            && (parts.iter()).all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The `.deb` of release `tag` (a `valid_tag`) when its `SHA256SUMS` lists it
/// (`<hash>  shagoff-commander_0.18.0_amd64.deb`): only that name, of that version and
/// architecture, goes into the download address.
pub fn deb_name(sums: &str, tag: &str) -> Option<String> {
    let want = format!("shagoff-commander_{}_amd64.deb", tag.strip_prefix('v')?);
    sums.lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .any(|n| n.trim_start_matches('*') == want)
        .then_some(want)
}

/// The SHA-256 of `name` in `sums`.
pub fn hash_of<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        let (h, n) = (w.next()?, w.next()?);
        (n.trim_start_matches('*') == name
            && h.len() == 64
            && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(h)
    })
}

/// The release key (minisign, id 982F950BAA31CF38; its secret half never leaves the author's machine, `just
/// sign-release`, 214a): `SHA256SUMS.minisig` must be its signature or nothing is installed. A
/// stolen GitHub token can publish a release, not one this program installs.
pub const PUBLIC_KEY: &str = "RWQ4zzGqC5UvmPKuX8YgZPMhka9C7qvJVPnYqLgqqIx1NhBj3A451IRE";

/// `sums` signed by `key` (`sig`: the `.minisig` file's text).
pub fn signed(sums: &[u8], sig: &str, key: &str) -> Result<(), String> {
    let bad = |_| crate::why::NO_SIGNATURE.to_string();
    let key = minisign_verify::PublicKey::from_base64(key).map_err(bad)?;
    let sig = minisign_verify::Signature::decode(sig).map_err(bad)?;
    key.verify(sums, &sig, false).map_err(bad)
}

/// `sha256sum -c --strict` of the `SHA256SUMS` in `dir` (the `.rpm` it lists is not downloaded):
/// Ok only when the files there match; else the line that says what did not.
pub fn verify(dir: &Path) -> Result<(), String> {
    let out = Command::new("sha256sum")
        .args(["-c", "--strict", "--ignore-missing", "SHA256SUMS"])
        .current_dir(dir)
        .env("LC_ALL", "C") // its `OK` / `FAILED` are translated otherwise
        .output()
        .map_err(|e| format!("sha256sum: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stdout)
            .lines()
            .chain(String::from_utf8_lossy(&out.stderr).lines())
            .find(|l| !l.ends_with(": OK"))
            .unwrap_or("SHA-256 does not match")
            .to_string()),
    }
}

fn fetch(url: &str, to: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "-m", "600", "-o"])
        .arg(to)
        .args(["--", url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Release `tag`'s signed `SHA256SUMS` (`signed`, before anything in it is trusted) and the
/// `.deb` it names into `dir` (curl, from this repository only), then `verify`. A package that
/// does not match is deleted. The package and its SHA-256. Blocking.
pub fn download(tag: &str, dir: &Path) -> Result<(PathBuf, String), String> {
    if !valid_tag(tag) {
        return Err(format!("bad tag {tag}"));
    }
    let base = format!("{REPO}/releases/download/{tag}");
    fetch(&format!("{base}/SHA256SUMS"), &dir.join("SHA256SUMS"))?;
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).map_err(|e| e.to_string())?;
    let sig = dir.join("SHA256SUMS.minisig");
    fetch(&format!("{base}/SHA256SUMS.minisig"), &sig)
        .map_err(|_| crate::why::NO_SIGNATURE.to_string())?;
    let sig = std::fs::read_to_string(sig).map_err(|_| crate::why::NO_SIGNATURE.to_string())?;
    signed(sums.as_bytes(), &sig, PUBLIC_KEY)?;
    let name = deb_name(&sums, tag).ok_or("SHA256SUMS: no package")?;
    let hash = hash_of(&sums, &name)
        .ok_or("SHA256SUMS: no package")?
        .to_string();
    let deb = dir.join(&name);
    fetch(&format!("{base}/{name}"), &deb)?;
    if let Err(e) = verify(dir) {
        let _ = std::fs::remove_file(&deb);
        return Err(e);
    }
    Ok((deb, hash))
}

/// Run as root by `install` with `deb hash name`: the package is copied into a dir only root can
/// write to and checked there, so the file installed is the one checked (214e: the one in
/// `~/.cache` is the user's to change between the check and the install).
pub const INSTALL_SCRIPT: &str = r#"set -e
d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
cp -- "$1" "$d/$3"
echo "$2  $d/$3" | sha256sum -c --strict --quiet -
apt-get install -y "$d/$3""#;

/// `pkexec` `INSTALL_SCRIPT` for the downloaded `.deb` (`deb_name`'s, of SHA-256 `hash`); pkexec
/// asks for the password itself. Blocking.
pub fn install(deb: &Path, hash: &str) -> Result<(), String> {
    let name = deb.file_name().ok_or("no package")?;
    let out = Command::new("pkexec")
        .args(["/bin/sh", "-c", INSTALL_SCRIPT, "sh"])
        .arg(deb)
        .arg(hash)
        .arg(name)
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

/// `CHANGELOG.md` as releases: (`0.138.0 — 2026-11-03`, its lines without the `- `), newest
/// first, at most `n`.
pub fn releases(md: &str, n: usize) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in md.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            if out.len() == n {
                break;
            }
            out.push((head.trim(), Vec::new()));
        } else if let Some(item) = line.strip_prefix("- ")
            && let Some((_, items)) = out.last_mut()
        {
            items.push(item.trim());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_releases_newest_first() {
        let md = "# Changelog\n\nIntro.\n- not a release\n\n## 0.2.0 — d2\n\n- feat: b\n- fix: c\n\n## 0.1.0 — d1\n\n- a\n";
        assert_eq!(
            releases(md, 10),
            [
                ("0.2.0 — d2", vec!["feat: b", "fix: c"]),
                ("0.1.0 — d1", vec!["a"])
            ]
        );
        assert_eq!(releases(md, 1).len(), 1);
        // The built-in one starts with this version (a release without its line would show stale).
        let real = releases(include_str!("../../../CHANGELOG.md"), 1);
        assert_eq!(real[0].0.split(' ').next(), Some(env!("CARGO_PKG_VERSION")));
    }

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

    /// A key made for these tests (`minisign -G -W`) and its signature of `SUMS`.
    const TEST_KEY: &str = "RWTT3cKIYkKL1VI9H4VmO1XqOK/H1y7Kcab5kFf2aiH35YnC/ftg5FR7";
    const SUMS: &str = "abc  shagoff-commander_0.1.0_amd64.deb\n";
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUTT3cKIYkKL1ZCdDDr2kJiRhcVNyfq4gielhG5dMipCQhzQ+3NB+e/n9Y11nKuClax8Lc77G3lgSLQBAMa54Q8tuNx1AbPMTgQ=
trusted comment: shagoff-commander v0.1.0
bKEqP+fmYtJTz2bPNzaDo8vkZFOCe3fMXSYn6Rhpq/UFzMUH5rc90LmG0DgJ+DAePJdyo9Yd0ldm3kf4XnHFBQ==
";

    #[test]
    fn regression_update_needs_the_signature_of_our_key() {
        assert_eq!(signed(SUMS.as_bytes(), SIG, TEST_KEY), Ok(()));
        let changed = SUMS.replace("abc", "abd");
        assert!(
            signed(changed.as_bytes(), SIG, TEST_KEY).is_err(),
            "changed after signing"
        );
        assert!(
            signed(SUMS.as_bytes(), SIG, PUBLIC_KEY).is_err(),
            "not by our key"
        );
        assert!(
            signed(SUMS.as_bytes(), "", TEST_KEY).is_err(),
            "no signature"
        );
        assert!(minisign_verify::PublicKey::from_base64(PUBLIC_KEY).is_ok());
    }

    #[test]
    fn regression_installed_package_is_the_checked_copy() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        // A stand-in apt-get: says what it was given and its bytes.
        let apt = bin.join("apt-get");
        std::fs::write(&apt, "#!/bin/sh\necho \"$3\"; cat \"$3\"\n").unwrap();
        std::fs::set_permissions(&apt, std::fs::Permissions::from_mode(0o755)).unwrap();
        let deb = d.path().join("p.deb");
        std::fs::write(&deb, "good\n").unwrap();
        let real = Command::new("sha256sum").arg(&deb).output().unwrap();
        let real = String::from_utf8_lossy(&real.stdout)[..64].to_string();
        let run = |hash: &str| {
            let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
            Command::new("sh")
                .args(["-c", INSTALL_SCRIPT, "sh"])
                .arg(&deb)
                .args([hash, "p.deb"])
                .env("PATH", path)
                .output()
                .unwrap()
        };
        let out = run(&real);
        assert!(out.status.success());
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(!said.contains(&*d.path().to_string_lossy()) && said.ends_with("/p.deb\ngood\n"));
        assert!(!run(&"0".repeat(64)).status.success()); // another file's hash: not installed
        assert_eq!(
            hash_of(&format!("{real}  *p.deb\n"), "p.deb"),
            Some(real.as_str())
        );
        assert_eq!(hash_of("short  p.deb\n", "p.deb"), None);
    }

    #[test]
    fn regression_update_takes_only_the_tags_amd64_deb() {
        let h = "a".repeat(64);
        let sums = format!(
            "{h}  shagoff-commander_0.118.0_arm64.deb\n{h}  shagoff-commander_0.118.0_amd64.deb\n{h}  shagoff-commander-0.118.0-1.x86_64.rpm\n"
        );
        assert_eq!(
            deb_name(&sums, "v0.118.0"),
            Some("shagoff-commander_0.118.0_amd64.deb".into())
        );
        assert_eq!(deb_name(&sums, "v0.119.0"), None); // another version's
        assert_eq!(deb_name(&format!("{h}  ../evil.deb"), "v0.118.0"), None);
        assert_eq!(deb_name("", "v0.118.0"), None);
        assert!(valid_tag("v0.118.0") && valid_tag("v10.0.12"));
        for bad in [
            "v0.118.0/../x",
            "latest",
            "0.1.0",
            "vv0.1.0",
            "v+1.0.0",
            "v1..0",
            "v1.0.0.0",
        ] {
            assert!(!valid_tag(bad), "{bad}");
        }
    }

    #[test]
    fn regression_update_refuses_a_package_that_does_not_match_its_hash() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("shagoff-commander_0.1.0_amd64.deb"),
            b"package",
        )
        .unwrap();
        // sha256 of "package"; the .rpm is listed but not there
        let ok = "bc4a71180870f7945155fbb02f4b0a2e3faa2a62d6d31b7039013055ed19869a";
        let sums = |h: &str| {
            format!(
                "{h}  shagoff-commander_0.1.0_amd64.deb\n{h}  shagoff-commander-0.1.0-1.x86_64.rpm\n"
            )
        };
        std::fs::write(d.path().join("SHA256SUMS"), sums(ok)).unwrap();
        assert_eq!(verify(d.path()), Ok(()));
        std::fs::write(d.path().join("SHA256SUMS"), sums(&"0".repeat(64))).unwrap();
        assert!(verify(d.path()).unwrap_err().contains("FAILED"));
    }
}
