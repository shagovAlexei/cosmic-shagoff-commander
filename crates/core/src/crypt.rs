//! Files → «Encrypt» / «Decrypt» (230): a file to and from OpenPGP by the `gpg` CLI, with a
//! password (symmetric, AES256) or to a public key of the user's keyring. `name` ↔ `name.gpg`
//! next to it; the output goes to a part file renamed without replacing. A password goes to gpg
//! on its stdin, never argv, and gpg-agent never caches it (`--no-symkey-cache`); a secret key's
//! own passphrase is gpg-agent's to ask (its pinentry).

use crate::why;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A public key that can encrypt: its fingerprint and its first user id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub fpr: String,
    pub uid: String,
}

/// How `encrypt` locks a file.
pub enum Lock<'a> {
    Password(&'a str),
    Key(&'a str),
}

/// Extensions `decrypt` takes off.
const EXTS: [&str; 3] = [".gpg", ".pgp", ".asc"];

/// The name decrypting `name` makes, if it has one of `EXTS`.
pub fn plain_name(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    EXTS.iter()
        .find(|e| lower.ends_with(*e) && name.len() > e.len())
        .map(|e| &name[..name.len() - e.len()])
}

fn gpg() -> Command {
    let mut c = crate::host::command("gpg");
    #[cfg(test)]
    c.arg("--homedir").arg(tests::home());
    c.args(["--batch", "--yes", "--no-tty"]);
    c
}

/// The public keys of the user's keyring that can encrypt (none without gpg).
/// ponytail: run on the UI thread when the dialog opens (tens of ms); off it if keyrings get big.
pub fn keys() -> Vec<Key> {
    let out = gpg().args(["--with-colons", "--list-keys"]).output();
    out.map(|o| parse_keys(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// `gpg --with-colons --list-keys` → the keys usable for encryption (field 12 has `E`), not
/// revoked / expired / invalid (field 2).
pub fn parse_keys(text: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut cur: Option<Key> = None; // a usable `pub` waiting for its fpr / uid
    for line in text.lines() {
        let f: Vec<&str> = line.split(':').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        match get(0) {
            "pub" => {
                keys.extend(cur.take().filter(|k| !k.fpr.is_empty()));
                let ok = !matches!(get(1), "r" | "e" | "i" | "d") && get(11).contains('E');
                cur = ok.then(|| Key {
                    fpr: String::new(),
                    uid: String::new(),
                });
            }
            "fpr" => {
                if let Some(k) = cur.as_mut().filter(|k| k.fpr.is_empty()) {
                    k.fpr = get(9).to_string();
                }
            }
            "uid" => {
                if let Some(k) = cur.as_mut().filter(|k| k.uid.is_empty()) {
                    k.uid = get(9).replace("\\x3a", ":");
                }
            }
            _ => {}
        }
    }
    keys.extend(cur.filter(|k| !k.fpr.is_empty()));
    keys
}

/// Whether `head` (a file's first bytes) starts with a password packet (SKESK, tag 3), so it
/// needs one typed. ASCII armor: the first byte of its base64 body.
pub fn wants_password(head: &[u8]) -> bool {
    if head.starts_with(b"-----BEGIN PGP") {
        return armored_first(head).is_some_and(|b| wants_password(&[b]));
    }
    match head.first() {
        Some(&b) if b & 0xc0 == 0xc0 => b & 0x3f == 3, // new format
        Some(&b) if b & 0x80 != 0 => (b >> 2) & 0x0f == 3, // old format
        _ => false,
    }
}

/// The first byte of an armored message's body: the first line after the blank one ending the
/// headers, its first two base64 characters.
fn armored_first(text: &[u8]) -> Option<u8> {
    let text = std::str::from_utf8(text).ok()?;
    let mut lines = text.lines().map(str::trim).skip_while(|l| !l.is_empty());
    let body = lines.find(|l| !l.is_empty())?.as_bytes();
    let v = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    Some(v(*body.first()?)? << 2 | v(*body.get(1)?)? >> 4)
}

/// `wants_password` of the file at `path` (its first 1 KB: room for an armor's headers).
pub fn file_wants_password(path: &Path) -> bool {
    let mut head = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| io::Read::read_to_end(&mut io::Read::take(f, 1024), &mut head))
        .is_ok_and(|_| wants_password(&head))
}

/// `f` on each of `sources` in turn: the files made and the ones that failed with why. Stops at
/// the first `Interrupted` (a key's passphrase not given: pinentry cancelled).
pub fn each(
    sources: &[PathBuf],
    mut f: impl FnMut(&Path) -> io::Result<PathBuf>,
) -> (Vec<PathBuf>, Vec<(PathBuf, io::Error)>) {
    let (mut made, mut bad) = (Vec::new(), Vec::new());
    for s in sources {
        match f(s) {
            Ok(p) => made.push(p),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => break,
            Err(e) => bad.push((s.clone(), e)),
        }
    }
    (made, bad)
}

/// `src` → `src.gpg` beside it.
pub fn encrypt(src: &Path, lock: &Lock) -> io::Result<PathBuf> {
    let mut out = src.as_os_str().to_owned();
    out.push(".gpg");
    let mut c = gpg();
    let password = match lock {
        Lock::Password(p) => {
            c.args(["--pinentry-mode", "loopback", "--no-symkey-cache"]);
            c.args([
                "--passphrase-fd",
                "0",
                "--cipher-algo",
                "AES256",
                "--symmetric",
            ]);
            Some(*p)
        }
        Lock::Key(fpr) => {
            // The key was picked from the user's own keyring: no web-of-trust question.
            c.args(["--trust-model", "always", "--encrypt", "--recipient", fpr]);
            None
        }
    };
    run(c, src, &PathBuf::from(out), password)
}

/// `src` (`name.gpg` …) → `name` beside it; `password` for a password-locked one, else
/// gpg-agent asks for the key's own.
pub fn decrypt(src: &Path, password: Option<&str>) -> io::Result<PathBuf> {
    let name = src.file_name().unwrap_or_default().to_string_lossy();
    let plain = plain_name(&name).ok_or_else(|| io::Error::other(why::NOT_ENCRYPTED))?;
    let out = src.with_file_name(plain);
    let mut c = gpg();
    if password.is_some() {
        c.args(["--pinentry-mode", "loopback", "--no-symkey-cache"]);
        c.args(["--passphrase-fd", "0"]);
    }
    // A signature that can't be checked (the sender's key unknown) makes gpg exit 2 though the
    // file was decrypted (and its integrity checked): its status line says so.
    c.args(["--status-fd", "1", "--decrypt"]);
    run(c, src, &out, password)
}

/// `c` writing `src`'s result to a part file, renamed to `out` (never replacing).
fn run(mut c: Command, src: &Path, out: &Path, password: Option<&str>) -> io::Result<PathBuf> {
    if std::fs::symlink_metadata(out).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    // Hidden, unique, made new by us (O_EXCL): gpg (`--yes`) only writes into it.
    let (part, _) = crate::ops::create_part(out)?;
    c.arg("--output").arg(crate::host::outside(&part));
    c.arg("--").arg(crate::host::outside(src));
    c.stdin(if password.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => io::Error::other(why::NO_GPG),
        _ => e,
    });
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    if let (Some(p), Some(mut stdin)) = (password, child.stdin.take()) {
        // gpg gone before reading it: its own error tells why.
        let _ = stdin.write_all(format!("{p}\n").as_bytes());
    }
    let o = child.wait_with_output()?;
    let status = String::from_utf8_lossy(&o.stdout);
    let decrypted =
        status.contains("[GNUPG:] DECRYPTION_OKAY") && !status.contains("DECRYPTION_FAILED");
    let done = if o.status.success() || decrypted {
        crate::ops::rename_noreplace(&part, out)
    } else {
        Err(gpg_error(&String::from_utf8_lossy(&o.stderr)))
    };
    if done.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    done.map(|_| out.to_path_buf())
}

/// gpg's stderr as an error: the usual ones as our own texts, else its last line.
fn gpg_error(err: &str) -> io::Error {
    let has = |s: &str| err.contains(s);
    // Cancelled first: gpg then also says "No secret key".
    if has("cancelled") {
        io::ErrorKind::Interrupted.into()
    } else if has("Bad passphrase") || has("Bad session key") {
        io::Error::other(why::BAD_PASSWORD)
    } else if has("No secret key") {
        io::Error::other(why::NO_SECRET_KEY)
    } else if has("no valid OpenPGP data") {
        io::Error::other(why::NOT_ENCRYPTED)
    } else {
        let last = err.lines().rev().find(|l| !l.trim().is_empty());
        io::Error::other(
            last.unwrap_or("gpg failed")
                .trim_start_matches("gpg: ")
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// A keyring of the tests' own: never the user's.
    pub(super) fn home() -> &'static Path {
        static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
        HOME.get_or_init(|| {
            let d = tempfile::Builder::new()
                .prefix("gpg")
                .tempdir_in("/tmp")
                .unwrap();
            let mut p = std::fs::metadata(d.path()).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut p, 0o700);
            std::fs::set_permissions(d.path(), p).unwrap();
            d
        })
        .path()
    }

    fn have_gpg() -> bool {
        crate::host::which("gpg").is_some()
    }

    /// Called at the end of each of the two gpg tests: the last one stops the keyring's agent
    /// (else one is left running per test run) and removes the keyring.
    fn done() {
        static LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2);
        if LEFT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            let _ = Command::new("gpgconf")
                .arg("--homedir")
                .arg(home())
                .args(["--kill", "gpg-agent"])
                .status();
            let _ = std::fs::remove_dir_all(home());
        }
    }

    #[test]
    fn plain_names() {
        assert_eq!(plain_name("a.txt.gpg"), Some("a.txt"));
        assert_eq!(plain_name("A.PGP"), Some("A"));
        assert_eq!(plain_name("x.asc"), Some("x"));
        assert_eq!(plain_name(".gpg"), None);
        assert_eq!(plain_name("a.txt"), None);
    }

    #[test]
    fn keys_from_colons() {
        let text = "\
tru::1:1700000000:0:3:1:5
pub:u:255:22:AAAA:1700000000:::u:::scESC:::::ed25519:::0:
fpr:::::::::FPR1:
uid:u::::1700000000::H1::Ann \\x3a Smith <ann@x>::::::::::0:
sub:u:255:18:BBBB:1700000000::::::e:::::cv25519::
fpr:::::::::FPR1SUB:
pub:r:255:22:CCCC:1700000000:::u:::scESC:::::ed25519:::0:
fpr:::::::::REVOKED:
uid:r::::::H2::Old <old@x>::::::::::0:
pub:u:255:22:DDDD:1700000000:::u:::scSC:::::ed25519:::0:
fpr:::::::::SIGNONLY:
uid:u::::::H3::Sign <s@x>::::::::::0:
";
        let k = parse_keys(text);
        assert_eq!(
            k,
            vec![Key {
                fpr: "FPR1".into(),
                uid: "Ann : Smith <ann@x>".into()
            }]
        );
    }

    #[test]
    fn password_packet_by_first_byte() {
        assert!(wants_password(&[0xc3])); // new format, tag 3
        assert!(wants_password(&[0x8c])); // old format, tag 3
        assert!(!wants_password(&[0x85])); // old format, tag 1 (key)
        assert!(!wants_password(&[0xc1]));
        assert!(!wants_password(b"-----BEGIN PGP MESSAGE"));
        // `gpg -ca` (password) and `gpg -ea` (key) bodies.
        assert!(wants_password(b"-----BEGIN PGP MESSAGE-----\n\njA0ECQMC\n"));
        assert!(!wants_password(
            b"-----BEGIN PGP MESSAGE-----\nComment: x\n\nhF4D\n"
        ));
        assert!(!wants_password(&[]));
    }

    #[test]
    fn password_round_trip() {
        if !have_gpg() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a b.txt");
        std::fs::write(&src, "secret").unwrap();
        let enc = encrypt(&src, &Lock::Password("pw 1")).unwrap();
        assert_eq!(enc, d.path().join("a b.txt.gpg"));
        assert!(file_wants_password(&enc));
        // Taken: never replaced.
        let e = encrypt(&src, &Lock::Password("pw 1")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        std::fs::remove_file(&src).unwrap();
        let e = decrypt(&enc, Some("wrong")).unwrap_err();
        assert_eq!(e.to_string(), why::BAD_PASSWORD);
        assert!(!src.exists());
        assert_eq!(decrypt(&enc, Some("pw 1")).unwrap(), src);
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "secret");
        let left: Vec<_> = std::fs::read_dir(d.path()).unwrap().collect();
        assert_eq!(left.len(), 2); // no part file
        done();
    }

    #[test]
    fn key_round_trip() {
        if !have_gpg() {
            return;
        }
        let made = gpg()
            .args(["--passphrase", "", "--quick-gen-key", "Test <t@x>"])
            .args(["default", "default", "never"])
            .output()
            .unwrap();
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        let k = keys();
        let key = k.iter().find(|k| k.uid == "Test <t@x>").unwrap();
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("k.bin");
        std::fs::write(&src, [0u8, 1, 2]).unwrap();
        let enc = encrypt(&src, &Lock::Key(&key.fpr)).unwrap();
        assert!(!file_wants_password(&enc));
        std::fs::remove_file(&src).unwrap();
        decrypt(&enc, None).unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), [0u8, 1, 2]);
        done();
    }

    #[test]
    fn each_stops_at_a_cancel_and_keeps_going_past_errors() {
        let src: Vec<PathBuf> = ["a", "b", "c", "d"].map(PathBuf::from).into();
        let (made, bad) = each(&src, |p| match p.to_str() {
            Some("b") => Err(io::Error::other(why::BAD_PASSWORD)),
            Some("c") => Err(io::ErrorKind::Interrupted.into()),
            _ => Ok(p.with_extension("gpg")),
        });
        assert_eq!(made, [PathBuf::from("a.gpg")]);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, PathBuf::from("b"));
    }

    #[test]
    fn cancelled_pinentry_is_not_a_missing_key() {
        let err = "gpg: public key decryption failed: Operation cancelled\ngpg: decryption failed: No secret key\n";
        assert_eq!(gpg_error(err).kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn not_encrypted_name_refused() {
        let e = decrypt(Path::new("/tmp/x.txt"), None).unwrap_err();
        assert_eq!(e.to_string(), why::NOT_ENCRYPTED);
    }
}
