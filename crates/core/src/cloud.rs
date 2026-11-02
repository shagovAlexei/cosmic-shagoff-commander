//! Cloud drives (215): OneDrive, Google Drive, Dropbox, Box through `rclone mount`. gvfs shows
//! Google Drive with object ids for names, so rclone it is: a FUSE dir with the real names, a
//! plain path for everything else. An account is an rclone remote (`~/.config/rclone/rclone.conf`);
//! a mount point `<root>/<plugin id>:<remote>` names its plugin, so drives and the switched-off
//! filter come from `/proc/self/mounts` alone.

use crate::drives::{self, Drive};
use crate::mount::{self, Error, Volume};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Debug, PartialEq, Eq)]
pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    /// rclone's storage type.
    pub kind: &'static str,
}

pub const PLUGINS: &[Plugin] = &[
    Plugin {
        id: "onedrive",
        name: "OneDrive",
        kind: "onedrive",
    },
    Plugin {
        id: "gdrive",
        name: "Google Drive",
        kind: "drive",
    },
    Plugin {
        id: "dropbox",
        name: "Dropbox",
        kind: "dropbox",
    },
    Plugin {
        id: "box",
        name: "Box",
        kind: "box",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub plugin: &'static Plugin,
}

/// The remotes of our plugins' types from `rclone listremotes --long` (`name:   type` lines).
pub fn parse_remotes(listremotes_long: &str) -> Vec<Remote> {
    listremotes_long
        .lines()
        .filter_map(|l| {
            let (name, kind) = l.rsplit_once(':')?;
            let plugin = PLUGINS.iter().find(|p| p.kind == kind.trim())?;
            Some(Remote {
                name: name.to_string(),
                plugin,
            })
        })
        .collect()
}

/// Every remote's name in `rclone listremotes --long`, of any type.
pub fn names(listremotes_long: &str) -> Vec<String> {
    (listremotes_long.lines())
        .filter_map(|l| Some(l.rsplit_once(':')?.0.to_string()))
        .collect()
}

/// A new account's name: the plugin's id, `id-2`, `id-3`… when taken by any remote (`rclone
/// config create` silently replaces one of the same name).
pub fn fresh_name(p: &Plugin, taken: &[String]) -> String {
    let free = |n: &str| !taken.iter().any(|t| t == n);
    if free(p.id) {
        return p.id.to_string();
    }
    (2..)
        .map(|i| format!("{}-{i}", p.id))
        .find(|n| free(n))
        .expect("a free name")
}

/// "OneDrive" for the account named after its plugin, else "OneDrive: work".
pub fn label(p: &Plugin, remote: &str) -> String {
    if remote == p.id {
        p.name.to_string()
    } else {
        format!("{}: {remote}", p.name)
    }
}

/// The mount point's name: `onedrive:work`.
pub fn point_name(r: &Remote) -> String {
    format!("{}:{}", r.plugin.id, r.name)
}

/// The plugin and remote a mount point's name stands for.
pub fn of_point(name: &str) -> Option<(&'static Plugin, &str)> {
    let (id, remote) = name.split_once(':')?;
    Some((PLUGINS.iter().find(|p| p.id == id)?, remote))
}

/// Where the clouds are mounted: `$XDG_RUNTIME_DIR/shagoff-clouds`.
pub fn root() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|r| PathBuf::from(r).join("shagoff-clouds"))
}

/// rclone mount points right under `root`, from `/proc/self/mounts`.
pub fn mounted(mounts: &str, root: &Path) -> Vec<PathBuf> {
    mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (_, point, fs) = (f.next()?, f.next()?, f.next()?);
            let point = drives::unescape(point);
            (fs == "fuse.rclone" && point.parent() == Some(root)).then_some(point)
        })
        .collect()
}

/// The accounts as volumes for Alt+F1 and the drive buttons: `device` is `rclone:` + the mount
/// point's name (`rclone:gdrive:work`), `mount` its `file://` point when mounted.
pub fn volumes(remotes: &[Remote], mounts: &str, root: &Path) -> Vec<Volume> {
    let points = mounted(mounts, root);
    remotes
        .iter()
        .map(|r| {
            let point = root.join(point_name(r));
            Volume {
                name: label(r.plugin, &r.name),
                device: format!("rclone:{}", point_name(r)),
                mount: points
                    .contains(&point)
                    .then(|| format!("file://{}", point.display())),
            }
        })
        .collect()
}

/// A drive button per mounted cloud of a plugin not switched off.
pub fn drives(mounts: &str, root: &Path, off: &[String]) -> Vec<Drive> {
    mounted(mounts, root)
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let (p, remote) = of_point(&name)?;
            if off.iter().any(|o| o == p.id) {
                return None;
            }
            Some(Drive {
                label: label(p, remote),
                path,
            })
        })
        .collect()
}

/// A cloud volume of a switched-off plugin.
pub fn hidden_volume(v: &Volume, off: &[String]) -> bool {
    (v.device.strip_prefix("rclone:"))
        .and_then(of_point)
        .is_some_and(|(p, _)| off.iter().any(|o| o == p.id))
}

/// `rclone` is in `PATH` (looked up, not run).
pub fn installed() -> bool {
    in_path("rclone")
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

fn rclone<const N: usize>(args: [&str; N]) -> Command {
    let mut c = Command::new("rclone");
    c.args(args);
    c
}

/// Run `c` with no stdin (with a pipe there, `rclone mount --daemon` never forks off and never
/// mounts — checked); stdout, or stderr as the error.
fn run(mut c: Command) -> Result<String, Error> {
    let out = c.output().map_err(|e| Error::Failed(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Our plugins' accounts; none without rclone or when it fails.
pub fn remotes() -> Vec<Remote> {
    if !installed() {
        return Vec::new();
    }
    run(rclone(["listremotes", "--long"]))
        .map(|s| parse_remotes(&s))
        .unwrap_or_default()
}

/// Sign in under a free name; that name. `rclone config create` opens the browser and waits for
/// the provider's answer (minutes, so no timeout of ours: `cancel` kills it). rclone writes the
/// remote before signing in, so a cancelled or failed one is removed again (else a remote with no
/// token is left) — safe, the name was nobody's.
pub fn add(p: &Plugin, cancel: &Arc<AtomicBool>) -> Result<String, Error> {
    // rclone waits for the browser on this port; taken, it fails with its whole usage text.
    // Checked first, so no remote is written either.
    if std::net::TcpListener::bind(AUTH_ADDR).is_err() {
        return Err(Error::Failed(PORT_BUSY.into()));
    }
    let name = fresh_name(p, &names(&run(rclone(["listremotes", "--long"]))?));
    let mut c = rclone(["config", "create", &name, p.kind]);
    // Its own process group (cancel kills it), so it would outlive us still waiting on the port
    // (the next sign-in then fails): it goes with the thread that started it, which waits for it.
    // SAFETY: prctl is async-signal-safe.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut c, || {
            rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM))
                .map_err(Into::into)
        });
    }
    let r = mount::talk_until(c, "", Some(cancel));
    if r.is_err() {
        let _ = forget(&name);
    }
    r.map(|_| name).map_err(|e| match e {
        Error::Failed(text) => Error::Failed(rclone_error(&text)),
        e => e,
    })
}

/// Where `rclone config create` waits for the provider's answer.
pub const AUTH_ADDR: &str = "127.0.0.1:53682";

/// `add`'s error when [`AUTH_ADDR`] is taken (the app shows its own text for it).
pub const PORT_BUSY: &str = "rclone auth port busy";

/// The gist of rclone's failure: its last `Error:` line without the timestamp, not the usage text
/// it prints after a failed command.
pub fn rclone_error(stderr: &str) -> String {
    let gist = (stderr.lines().rev())
        .filter_map(|l| l.split_once("Error: ").or_else(|| l.split_once("error: ")))
        .map(|(_, e)| e.trim())
        .next();
    gist.unwrap_or(stderr.trim()).to_string()
}

/// Remove an account: unmount it first (a mount with no account would linger), then forget it.
/// Not forgotten if it cannot be unmounted (busy).
pub fn remove(name: &str) -> Result<(), Error> {
    if let Some(root) = root() {
        let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        for point in mounted(&mounts, &root) {
            let ours = (point.file_name())
                .is_some_and(|n| of_point(&n.to_string_lossy()).is_some_and(|(_, r)| r == name));
            if ours {
                unmount(&point)?;
            }
        }
    }
    forget(name)
}

/// Remove an account (its token only: the files in the cloud stay).
pub fn forget(name: &str) -> Result<(), Error> {
    run(rclone(["config", "delete", name])).map(drop)
}

/// Mount the account a point name stands for (`gdrive:work`); its mount point.
pub fn mount(point_name: &str) -> Result<PathBuf, Error> {
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    let root = root().ok_or_else(|| Error::Failed("no XDG_RUNTIME_DIR".into()))?;
    let (_, name) =
        of_point(point_name).ok_or_else(|| Error::Failed(format!("{point_name}: not a cloud")))?;
    let point = root.join(point_name);
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    if mounted(&mounts, &root).contains(&point) {
        // Mounted already (another window, the list a moment old): just go there.
        if std::fs::read_dir(&point).is_ok() {
            return Ok(point);
        }
        // Left by a crashed rclone ("Transport endpoint is not connected"): mount again.
        let _ = fusermount(&point, true);
    }
    std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700)
        .create(&point)
        .map_err(failed)?;
    let target = format!("{name}:");
    let mut c = rclone(["mount", &target]);
    c.arg(&point)
        .args(["--daemon", "--vfs-cache-mode", "writes"]);
    run(c).map(|_| point)
}

/// Unmount a cloud and remove its mount point.
pub fn unmount(point: &Path) -> Result<(), Error> {
    fusermount(point, false)?;
    let _ = std::fs::remove_dir(point);
    Ok(())
}

fn fusermount(point: &Path, lazy: bool) -> Result<(), Error> {
    // fuse3 has only `fusermount3` on Fedora; Ubuntu links `fusermount` to it.
    let bin = if in_path("fusermount3") {
        "fusermount3"
    } else {
        "fusermount"
    };
    let mut c = Command::new(bin);
    c.arg(if lazy { "-uz" } else { "-u" }).arg(point);
    run(c).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REMOTES: &str = "gdrive:   drive\nmy drive: drive\nnas:      sftp\nonedrive: onedrive\n";

    #[test]
    fn parse_remotes_keeps_plugin_kinds() {
        let r = parse_remotes(REMOTES);
        let got: Vec<_> = r.iter().map(|r| (r.name.as_str(), r.plugin.id)).collect();
        assert_eq!(
            got,
            [
                ("gdrive", "gdrive"),
                ("my drive", "gdrive"),
                ("onedrive", "onedrive")
            ]
        );
    }

    #[test]
    fn fresh_name_skips_any_taken_name() {
        let taken = names(REMOTES);
        assert_eq!(taken, ["gdrive", "my drive", "nas", "onedrive"]);
        assert_eq!(fresh_name(&PLUGINS[0], &taken), "onedrive-2");
        assert_eq!(fresh_name(&PLUGINS[2], &taken), "dropbox");
        // An sftp remote named like the plugin is not replaced by the sign-in (review).
        let taken = names("box: sftp\nbox-2: s3\n");
        assert_eq!(fresh_name(&PLUGINS[3], &taken), "box-3");
    }

    #[test]
    fn rclone_error_drops_the_usage_text() {
        let stderr = "Error: config failed to refresh token: failed to start auth webserver: \
                      listen tcp 127.0.0.1:53682: bind: address already in use\nUsage:\n  \
                      rclone config create name type [key value]* [flags]\n\nFlags:\n  \
                      --all   Ask the full set of config questions\n\n\
                      2026/11/02 21:05:05 Fatal error: config failed to refresh token: failed to \
                      start auth webserver: listen tcp 127.0.0.1:53682: bind: address already in use\n";
        assert_eq!(
            rclone_error(stderr),
            "config failed to refresh token: failed to start auth webserver: listen tcp \
             127.0.0.1:53682: bind: address already in use"
        );
        assert_eq!(rclone_error("plain failure\n"), "plain failure");
    }

    #[test]
    fn labels_and_points() {
        let r = parse_remotes(REMOTES);
        assert_eq!(label(r[0].plugin, "gdrive"), "Google Drive");
        assert_eq!(label(r[1].plugin, "my drive"), "Google Drive: my drive");
        assert_eq!(point_name(&r[1]), "gdrive:my drive");
        let got = of_point("gdrive:my drive").map(|(p, n)| (p.id, n));
        assert_eq!(got, Some(("gdrive", "my drive")));
        assert!(of_point("sftp:host=nas").is_none());
    }

    #[test]
    fn volumes_and_drives_from_mounts() {
        let root = Path::new("/run/user/1000/shagoff-clouds");
        // As the kernel writes it (checked 2026-11-02), a space as `\040`.
        let mounts = "gdrive: /run/user/1000/shagoff-clouds/gdrive:gdrive fuse.rclone \
                      rw,nosuid,nodev,relatime,user_id=1000,group_id=1000 0 0\n\
                      my\\040drive: /run/user/1000/shagoff-clouds/gdrive:my\\040drive \
                      fuse.rclone rw 0 0\n\
                      /dev/sda1 /media/x ext4 rw 0 0\n\
                      x: /tmp/elsewhere fuse.rclone rw 0 0\n";
        let v = volumes(&parse_remotes(REMOTES), mounts, root);
        assert_eq!(v[0].device, "rclone:gdrive:gdrive");
        assert_eq!(
            v[0].mount.as_deref(),
            Some("file:///run/user/1000/shagoff-clouds/gdrive:gdrive")
        );
        assert!(v[1].mount.is_some()); // my drive
        assert_eq!(v[2].mount, None); // onedrive
        let d = drives(mounts, root, &[]);
        let labels: Vec<_> = d.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["Google Drive", "Google Drive: my drive"]);
        assert!(drives(mounts, root, &["gdrive".into()]).is_empty());
    }

    #[test]
    fn hidden_only_for_switched_off() {
        let off = vec!["gdrive".to_string()];
        let v = |d: &str| Volume {
            name: String::new(),
            device: d.into(),
            mount: None,
        };
        assert!(hidden_volume(&v("rclone:gdrive:my drive"), &off));
        assert!(!hidden_volume(&v("rclone:onedrive:onedrive"), &off));
        assert!(!hidden_volume(&v("/dev/sdb1"), &off));
    }
}
