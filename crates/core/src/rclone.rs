//! Our own rclone (216d): the official build from rclone.org in
//! `$XDG_DATA_HOME/shagoff-commander/rclone/rclone`, used in place of the distribution's (Ubuntu
//! 24.04 has 1.60: no MEGA, Proton Drive, iCloud). Its `SHA256SUMS` is checked with `gpgv` against
//! rclone's signing key built in (`res/rclone-keys.gpg`, fingerprint
//! FBF737ECE9F8AB18604BD2AC93935E02FF3B54FA — the same at rclone.org/KEYS, keys.openpgp.org and
//! keyserver.ubuntu.com, 2026-11-03); only the signed text is read, then the zip's SHA-256.

use crate::mount::Error;
use sha2::Digest;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// rclone's release signing keys (dearmored `rclone.org/KEYS`).
const KEYRING: &[u8] = include_bytes!("../../../res/rclone-keys.gpg");

const DOWNLOADS: &str = "https://downloads.rclone.org";

/// Where ours lives (`…/shagoff-commander/rclone`, with `rclone` and `VERSION` in it).
pub fn dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| Path::new(d).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    Some(data.join("shagoff-commander/rclone"))
}

/// Our rclone's version (`v1.75.2`, `?` without its `VERSION`) when it is there: whenever
/// `program` runs ours.
pub fn own() -> Option<String> {
    let dir = dir()?;
    dir.join("rclone").is_file().then_some(())?;
    let v = std::fs::read_to_string(dir.join("VERSION")).unwrap_or_default();
    Some(
        Some(v.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("?")
            .to_string(),
    )
}

/// The program to run: ours if there, else `rclone` from `PATH`.
pub fn program() -> PathBuf {
    (dir().map(|d| d.join("rclone")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "rclone".into())
}

/// rclone's name for this machine in its zips.
fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        "x86" => Some("386"),
        "arm" => Some("arm-v7"),
        _ => None,
    }
}

/// `rclone v1.75.2` (downloads.rclone.org/version.txt) → `v1.75.2`; nothing else goes into a URL.
pub fn parse_version(text: &str) -> Option<String> {
    let v = text.trim().strip_prefix("rclone ")?;
    let plain =
        v.starts_with('v') && v.len() > 1 && v[1..].chars().all(|c| c.is_ascii_digit() || c == '.');
    plain.then(|| v.to_string())
}

/// The zip of `version` for `arch`: `rclone-v1.75.2-linux-amd64.zip`.
fn zip_name(version: &str, arch: &str) -> String {
    format!("rclone-{version}-linux-{arch}.zip")
}

/// The SHA-256 a signed `SHA256SUMS` text gives for `name`.
pub fn hash_of<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once("  ")?;
        (file.trim() == name && hash.len() == 64).then_some(hash)
    })
}

fn fetch(url: &str) -> Result<Vec<u8>, Error> {
    let out = Command::new("curl")
        .args(["-fsSL", "-m", "600", "--", url])
        .output()
        .map_err(|e| Error::Failed(format!("curl: {e}")))?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(Error::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )),
    }
}

/// The text `gpgv` vouches for, with our keyring in `tmp`: what is outside the signature is
/// never read.
fn signed(sums: &[u8], tmp: &Path) -> Result<String, Error> {
    let keyring = tmp.join("keys.gpg");
    let file = tmp.join("SHA256SUMS");
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    std::fs::write(&keyring, KEYRING).map_err(failed)?;
    std::fs::write(&file, sums).map_err(failed)?;
    let out = Command::new("gpgv")
        .arg("--keyring")
        .arg(&keyring)
        .args(["--output", "-", "--"])
        .arg(&file)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| Error::Failed(format!("gpgv: {e}")))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        false => Err(Error::Failed(format!(
            "SHA256SUMS: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("bad signature")
        ))),
    }
}

/// The `rclone` program out of the release zip.
fn unzip(zip: &[u8], version: &str, arch: &str) -> Result<Vec<u8>, Error> {
    let failed = |e: zip::result::ZipError| Error::Failed(e.to_string());
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(failed)?;
    let name = format!("rclone-{version}-linux-{arch}/rclone");
    let mut f = a.by_name(&name).map_err(failed)?;
    let mut out = Vec::new();
    f.read_to_end(&mut out)
        .map_err(|e| Error::Failed(e.to_string()))?;
    Ok(out)
}

/// Download the latest official rclone, check it, put it in `dir`: its version. Blocking.
pub fn install() -> Result<String, Error> {
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    let arch = (arch()).ok_or_else(|| {
        Error::Failed(format!(
            "rclone.org: no build for {}",
            std::env::consts::ARCH
        ))
    })?;
    let dir = dir().ok_or_else(|| Error::Failed("no HOME".into()))?;
    let text = String::from_utf8_lossy(&fetch(&format!("{DOWNLOADS}/version.txt"))?).into_owned();
    let version =
        parse_version(&text).ok_or_else(|| Error::Failed(format!("version.txt: {text}")))?;
    if own().as_deref() == Some(version.as_str()) {
        return Ok(version); // the latest already
    }
    std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700)
        .create(&dir)
        .map_err(failed)?;
    // Left by an install that never ended (closed, crashed): tens of MB each.
    for e in std::fs::read_dir(&dir).map_err(failed)?.flatten() {
        if e.file_name().to_string_lossy().starts_with(".get-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let tmp = dir.join(format!(".get-{}", std::process::id()));
    std::fs::create_dir(&tmp).map_err(failed)?;
    let r = (|| {
        let sums = fetch(&format!("{DOWNLOADS}/{version}/SHA256SUMS"))?;
        let sums = signed(&sums, &tmp)?;
        let name = zip_name(&version, arch);
        let want =
            hash_of(&sums, &name).ok_or_else(|| Error::Failed(format!("SHA256SUMS: no {name}")))?;
        let zip = fetch(&format!("{DOWNLOADS}/{version}/{name}"))?;
        let got: String = (sha2::Sha256::digest(&zip).iter())
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != want {
            return Err(Error::Failed(format!("{name}: SHA-256 does not match")));
        }
        let program = unzip(&zip, &version, arch)?;
        let part = tmp.join("rclone");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(failed)?;
        f.write_all(&program).map_err(failed)?;
        drop(f);
        std::fs::set_permissions(&part, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .map_err(failed)?;
        // The program first: VERSION never names one that is not there.
        std::fs::rename(&part, dir.join("rclone")).map_err(failed)?;
        std::fs::write(tmp.join("VERSION"), &version).map_err(failed)?;
        std::fs::rename(tmp.join("VERSION"), dir.join("VERSION")).map_err(failed)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r.map(|_| version)
}

/// `remove`'s refusal while a cloud is mounted (the app shows its own text for it).
pub const CLOUDS_MOUNTED: &str = "clouds mounted";

/// Remove ours: the distribution's is used again. Refused while a cloud is mounted: its rclone
/// would run on with no way left here to unmount it.
pub fn remove() -> Result<(), Error> {
    let dir = dir().ok_or_else(|| Error::Failed("no HOME".into()))?;
    if let Some(root) = crate::cloud::root() {
        let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        if !crate::cloud::mounted(&mounts, &root).is_empty() {
            return Err(Error::Failed(CLOUDS_MOUNTED.into()));
        }
    }
    std::fs::remove_dir_all(dir).map_err(|e| Error::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_txt_is_taken_only_when_plain() {
        assert_eq!(
            parse_version("rclone v1.75.2\n").as_deref(),
            Some("v1.75.2")
        );
        assert_eq!(parse_version("rclone v1.75.2/../x"), None);
        assert_eq!(parse_version("<html>"), None);
        assert_eq!(parse_version("rclone v"), None);
    }

    #[test]
    fn hash_of_finds_the_zip() {
        let sums = "ab  rclone-v1-linux-amd64.zip\n\
                    349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702  \
                    rclone-v1.75.2-linux-amd64.zip\n";
        assert_eq!(
            hash_of(sums, &zip_name("v1.75.2", "amd64")),
            Some("349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702")
        );
        assert_eq!(hash_of(sums, "rclone-v1-linux-amd64.zip"), None); // not a SHA-256
    }

    /// The whole way over the network, into `$XDG_DATA_HOME` (run by hand: `XDG_DATA_HOME=/tmp/x
    /// cargo test -p shagoff-core install_for_real -- --ignored`); nothing is run.
    #[test]
    #[ignore]
    fn install_for_real() {
        let v = install().unwrap();
        assert_eq!(own(), Some(v));
        assert!(std::fs::metadata(program()).unwrap().len() > 10_000_000);
    }

    /// The real `SHA256SUMS` of rclone 1.75.2 checks with the built-in key; one changed hash in
    /// it does not (needs `gpgv`, there on every Debian / Ubuntu / Fedora: apt and rpm use it).
    #[test]
    fn signed_sums_check_and_a_changed_one_does_not() {
        if Command::new("gpgv").arg("--version").output().is_err() {
            return;
        }
        let sums = include_str!("testdata/rclone-SHA256SUMS");
        let tmp = std::env::temp_dir().join(format!("shagoff-rclone-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let text = signed(sums.as_bytes(), &tmp).unwrap();
        assert!(!text.contains("BEGIN PGP"));
        assert_eq!(
            hash_of(&text, "rclone-v1.75.2-linux-amd64.zip"),
            Some("349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702")
        );
        let changed = sums.replace("349ac8", "349ac9");
        assert!(signed(changed.as_bytes(), &tmp).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
