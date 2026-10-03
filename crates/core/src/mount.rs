//! USB sticks, partitions and network shares through the `gio` CLI (udisks + gvfs): no glib in
//! the build. gvfs shows every network mount as a dir under `$XDG_RUNTIME_DIR/gvfs` (FUSE), so
//! once mounted it is a plain path for everything else.

use crate::drives::Drive;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A volume with a unix device that udisks can mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub device: String,
    pub mount: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// gio asked for a password and the field was empty.
    NeedPassword,
    /// gio asked for the password a second time.
    WrongPassword,
    /// Any other question (unknown host key, …): its text.
    Question(String),
    Failed(String),
}

/// Volumes with a unix device and `can_mount=1`, from `gio mount -li`.
pub fn volumes(gio_list: &str) -> Vec<Volume> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut out = Vec::new();
    // (indent of the Volume line, volume, can_mount)
    let mut cur: Option<(usize, Volume, bool)> = None;
    let mut flush = |cur: &mut Option<(usize, Volume, bool)>| {
        if let Some((_, v, true)) = cur.take()
            && !v.device.is_empty()
        {
            out.push(v);
        }
    };
    for line in gio_list.lines() {
        let t = line.trim_start();
        if cur.as_ref().is_some_and(|(i, ..)| indent(line) <= *i) {
            flush(&mut cur);
        }
        if let Some(rest) = t.strip_prefix("Volume(") {
            let name = rest.split_once("): ").map_or("", |(_, n)| n).to_string();
            let v = Volume {
                name,
                device: String::new(),
                mount: None,
            };
            cur = Some((indent(line), v, false));
        } else if let Some((_, v, can)) = &mut cur {
            if let Some(d) = t.strip_prefix("unix-device: ") {
                v.device = d.trim_matches('\'').to_string();
            } else if t == "can_mount=1" {
                *can = true;
            } else if t.starts_with("Mount(") {
                v.mount = t.split_once(" -> file://").map(|(_, u)| uri_path(u));
            }
        }
    }
    flush(&mut cur);
    out
}

/// The path of a `file://` URI (already without the scheme): `%XX` decoded.
fn uri_path(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(byte) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    OsString::from_vec(out).into()
}

/// `sftp:host=nas,user=bob` → `nas`; `smb-share:server=nas,share=media` → `nas/media`.
pub fn gvfs_label(name: &str) -> String {
    let Some((_, params)) = name.split_once(':') else {
        return name.to_string();
    };
    let get = |key: &str| {
        params
            .split(',')
            .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    };
    match (get("host").or(get("server")), get("share")) {
        (Some(h), Some(s)) => format!("{h}/{s}"),
        (Some(h), None) => h.to_string(),
        _ => name.to_string(),
    }
}

/// Drive buttons for the gvfs mounts: the entries of `root` (`$XDG_RUNTIME_DIR/gvfs`), by name.
pub fn gvfs_drives(root: &Path) -> Vec<Drive> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<OsString> = rd.flatten().map(|e| e.file_name()).collect();
    names.sort();
    names
        .into_iter()
        .map(|n| Drive {
            label: gvfs_label(&n.to_string_lossy()),
            path: root.join(n),
        })
        .collect()
}

/// `local path: /run/user/1000/gvfs/…` from `gio info`.
pub fn local_path(gio_info: &str) -> Option<PathBuf> {
    gio_info
        .lines()
        .find_map(|l| l.strip_prefix("local path: "))
        .map(PathBuf::from)
}

/// The reply to a `gio mount` prompt (the last output line, ending in `": "`). User and domain
/// take the default gio offers (from the URL); the password is sent once: gio re-asks forever.
pub fn answer(prompt: &str, password: &str, asked: &mut bool) -> Result<String, Error> {
    if prompt.starts_with("User") || prompt.starts_with("Domain") {
        Ok(String::new())
    } else if prompt.starts_with("Password") {
        if *asked {
            Err(Error::WrongPassword)
        } else if password.is_empty() {
            Err(Error::NeedPassword)
        } else {
            *asked = true;
            Ok(password.to_string())
        }
    } else {
        Err(Error::Question(prompt.to_string()))
    }
}

/// Run `cmd`, answering its prompts; stdout on success.
fn talk(mut cmd: Command, password: &str) -> Result<String, Error> {
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    let (mut stdin, mut stdout) = (child.stdin.take(), child.stdout.take().expect("piped"));
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    // `since`: output after our last reply (a prompt answered with echo gets no newline).
    let (mut out, mut since, mut buf, mut asked) = (Vec::new(), 0, [0; 4096], false);
    let aborted = loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break None,
            Ok(n) => n,
        };
        out.extend_from_slice(&buf[..n]);
        if !out.ends_with(b": ") {
            continue;
        }
        let text = String::from_utf8_lossy(&out[since..]).into_owned();
        let prompt = text.rsplit('\n').next().unwrap_or_default();
        since = out.len();
        match answer(prompt, password, &mut asked) {
            Ok(reply) => {
                let sent = stdin
                    .as_mut()
                    .map(|s| s.write_all(format!("{reply}\n").as_bytes()));
                if !matches!(sent, Some(Ok(()))) {
                    break None;
                }
            }
            Err(Error::Question(_)) => break Some(Error::Question(text.trim().to_string())),
            Err(e) => break Some(e),
        }
    };
    drop(stdin);
    if let Some(e) = aborted {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    let status = child.wait().map_err(failed)?;
    let errors = errors.join().unwrap_or_default();
    let out = String::from_utf8_lossy(&out).into_owned();
    if status.success() {
        Ok(out)
    } else {
        let msg = if errors.trim().is_empty() {
            &out
        } else {
            &errors
        };
        Err(Error::Failed(msg.trim().to_string()))
    }
}

/// gio with English prompts (we parse them) and no stdin of ours.
fn gio<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(args: I) -> Command {
    let mut c = Command::new("gio");
    c.args(args)
        .env_remove("LC_ALL")
        .env_remove("LANGUAGE")
        .env("LC_MESSAGES", "C");
    c
}

/// The volumes gio knows (`gio mount -li`).
pub fn list() -> Result<Vec<Volume>, Error> {
    talk(gio(["mount", "-li"]), "").map(|s| volumes(&s))
}

/// Mount a partition by device; its mount point.
pub fn mount_device(device: &str) -> Result<PathBuf, Error> {
    talk(gio(["mount", "-d", device]), "")?;
    list()?
        .into_iter()
        .find(|v| v.device == device)
        .and_then(|v| v.mount)
        .ok_or_else(|| Error::Failed(format!("{device}: no mount point")))
}

/// Mount a network location (`sftp://user@host/dir`, `smb://…`); its local (FUSE) path.
pub fn connect(url: &str, password: &str) -> Result<PathBuf, Error> {
    // "Already mounted" is an error to gio but fine for us: the local path decides.
    let mounted = talk(gio(["mount", url]), password);
    match talk(gio(["info", url]), "")
        .ok()
        .and_then(|s| local_path(&s))
    {
        Some(p) => Ok(p),
        None => Err(mounted.err().unwrap_or_else(|| {
            Error::Failed(format!("{url}: no local path (gvfs-fuse not running?)"))
        })),
    }
}

/// Eject (sticks: also powers the drive off), else plain unmount (partitions, network).
pub fn unmount(path: &Path) -> Result<(), Error> {
    let arg = |flag: &'static str| [OsStr::new("mount"), OsStr::new(flag), path.as_os_str()];
    talk(gio(arg("-e")), "")
        .or_else(|_| talk(gio(arg("-u")), ""))
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from this machine's `gio mount -li`, plus a stick and a volume without a device.
    const LIST: &str = "\
Drive(0): SAMSUNG MZVLB1T0HBLR-00000
  Type: GProxyDrive (GProxyVolumeMonitorUDisks2)
  ids:
   unix-device: '/dev/nvme0n1'
  can_eject=0
  Volume(0): sys
    Type: GProxyVolume (GProxyVolumeMonitorUDisks2)
    ids:
     class: 'device'
     unix-device: '/dev/nvme0n1p3'
     label: 'sys'
    can_mount=1
    can_eject=0
    Mount(0): sys -> file:///media/shag/sys
      Type: GProxyMount (GProxyVolumeMonitorUDisks2)
      can_unmount=1
Drive(2): AI Mass Storage
  ids:
   unix-device: '/dev/sda'
  can_eject=1
  Volume(0): MY STICK
    ids:
     unix-device: '/dev/sda1'
    can_mount=1
  Volume(1): locked
    ids:
     unix-device: '/dev/sda2'
    can_mount=0
Volume(0): Phone
  Type: GProxyVolume (GProxyVolumeMonitorMTP)
  ids:
   unix-device: '/dev/bus/usb/003/004'
  can_mount=1
Volume(1): cdda
  can_mount=1
Mount(0): nas -> sftp://nas/
";

    #[test]
    fn volumes_mounted_and_not() {
        let v = volumes(LIST);
        let got: Vec<(&str, &str, Option<&Path>)> = v
            .iter()
            .map(|v| (v.name.as_str(), v.device.as_str(), v.mount.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("sys", "/dev/nvme0n1p3", Some(Path::new("/media/shag/sys"))),
                ("MY STICK", "/dev/sda1", None),
                ("Phone", "/dev/bus/usb/003/004", None),
            ]
        );
    }

    #[test]
    fn volumes_percent_decoded() {
        let list = "Volume(0): My Disk\n  unix-device: '/dev/sdb1'\n  can_mount=1\n  \
                    Mount(0): My Disk -> file:///media/shag/My%20Disk\n";
        assert_eq!(
            volumes(list)[0].mount.as_deref(),
            Some(Path::new("/media/shag/My Disk"))
        );
    }

    #[test]
    fn gvfs_labels() {
        assert_eq!(gvfs_label("sftp:host=nas,user=bob"), "nas");
        assert_eq!(gvfs_label("smb-share:server=nas,share=media"), "nas/media");
        assert_eq!(
            gvfs_label("ftp:host=ftp.example.org,port=2121"),
            "ftp.example.org"
        );
        assert_eq!(gvfs_label("weird"), "weird");
    }

    #[test]
    fn gvfs_drives_from_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sftp:host=b")).unwrap();
        std::fs::create_dir(d.path().join("dav:host=a,ssl=true")).unwrap();
        let got = gvfs_drives(d.path());
        let labels: Vec<&str> = got.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["a", "b"]);
        assert_eq!(got[1].path, d.path().join("sftp:host=b"));
        assert!(gvfs_drives(&d.path().join("missing")).is_empty());
    }

    #[test]
    fn local_path_from_info() {
        let info = "display name: / on nas\nlocal path: /run/user/1000/gvfs/sftp:host=nas\n\
                    uri: sftp://nas/\n";
        assert_eq!(
            local_path(info),
            Some(PathBuf::from("/run/user/1000/gvfs/sftp:host=nas"))
        );
        assert_eq!(local_path("uri: sftp://nas/\n"), None);
    }

    #[test]
    fn answers() {
        let mut asked = false;
        assert_eq!(answer("User [bob]: ", "pw", &mut asked), Ok(String::new()));
        assert_eq!(
            answer("Domain [WORKGROUP]: ", "pw", &mut asked),
            Ok(String::new())
        );
        assert_eq!(answer("Password: ", "pw", &mut asked), Ok("pw".into()));
        assert_eq!(
            answer("Password: ", "pw", &mut asked),
            Err(Error::WrongPassword)
        );
        assert_eq!(
            answer("Password: ", "", &mut false),
            Err(Error::NeedPassword)
        );
        assert!(matches!(
            answer("Choice: ", "pw", &mut false),
            Err(Error::Question(_))
        ));
    }

    /// A stand-in for gio: a shell script.
    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[test]
    fn talk_sends_password_once() {
        let script = r#"printf 'Authentication Required\nUser [bob]: '; read u
            printf 'Password: '; read p; [ "$u" = "" ] && [ "$p" = "s3cret" ] && echo ok"#;
        assert!(
            talk(sh(script), "s3cret")
                .unwrap()
                .ends_with("Password: ok\n")
        );
    }

    #[test]
    fn talk_wrong_password_stops() {
        let script = "while :; do printf 'Password: '; read p || exit 1; done";
        assert_eq!(talk(sh(script), "bad"), Err(Error::WrongPassword));
    }

    #[test]
    fn talk_question_aborts() {
        let script = "printf 'The identity of host is unknown\\n[1] Log In Anyway\\n[2] Cancel\\nChoice: '; read c; echo answered";
        let Err(Error::Question(q)) = talk(sh(script), "pw") else {
            panic!("not a question")
        };
        assert!(q.contains("identity of host"));
    }

    #[test]
    fn talk_failure_reports_stderr() {
        assert_eq!(
            talk(sh("echo 'gio: nas: Connection refused' >&2; exit 2"), ""),
            Err(Error::Failed("gio: nas: Connection refused".into()))
        );
        assert_eq!(
            talk(sh("printf 'Password: '; read p"), ""),
            Err(Error::NeedPassword)
        );
    }
}
