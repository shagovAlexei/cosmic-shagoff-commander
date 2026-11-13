//! Cloud drives (215, 216) through `rclone mount`: the services of `PLUGINS`, signed in in the
//! browser or through fields and rclone's questions (`create` / `answer`). gvfs shows
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
    /// Signs in in the browser (OAuth); else through `fields` and rclone's questions (216b).
    pub browser: bool,
    /// Options given to `rclone config create` as they are.
    pub preset: &'static [(&'static str, &'static str)],
    /// Options asked before creating the account.
    pub fields: &'static [Field],
}

/// An rclone option the user types; the app labels it `cloud-field-<key>`.
#[derive(Debug, PartialEq, Eq)]
pub struct Field {
    pub key: &'static str,
    pub secret: bool,
    pub default: &'static str,
}

const fn field(key: &'static str) -> Field {
    Field {
        key,
        secret: false,
        default: "",
    }
}

const fn secret(key: &'static str) -> Field {
    Field {
        key,
        secret: true,
        default: "",
    }
}

const fn oauth(id: &'static str, name: &'static str, kind: &'static str) -> Plugin {
    Plugin {
        id,
        name,
        kind,
        browser: true,
        preset: &[],
        fields: &[],
    }
}

const fn typed(
    id: &'static str,
    name: &'static str,
    kind: &'static str,
    fields: &'static [Field],
) -> Plugin {
    Plugin {
        id,
        name,
        kind,
        browser: false,
        preset: &[],
        fields,
    }
}

pub const PLUGINS: &[Plugin] = &[
    oauth("onedrive", "OneDrive", "onedrive"),
    oauth("gdrive", "Google Drive", "drive"),
    oauth("dropbox", "Dropbox", "dropbox"),
    oauth("box", "Box", "box"),
    // rclone's shared app is refused by Yandex (410 Gone, since 10.2026): the user's own, signed
    // in by the code Yandex shows (`yandex_code`).
    typed(
        "yandex",
        "Yandex Disk",
        "yandex",
        &[field("client_id"), field("client_secret")],
    ),
    oauth("pcloud", "pCloud", "pcloud"),
    // rclone's shared app has used up Google's daily upload quota: the user's own, if given.
    // ponytail: its secret is on rclone's argv while the browser sign-in waits (Google calls a
    // desktop app's secret not confidential); rclone.conf written by us if that ever matters.
    Plugin {
        fields: &[field("client_id"), field("client_secret")],
        ..oauth("gphotos", "Google Photos", "gphotos")
    },
    oauth("hidrive", "HiDrive", "hidrive"),
    // An app password (Mail.ru → Security → App passwords): the account's own one is refused.
    typed(
        "mailru",
        "Mail.ru Cloud",
        "mailru",
        &[field("user"), secret("pass")],
    ),
    Plugin {
        preset: &[("provider", "Other")],
        ..typed(
            "s3",
            "S3",
            "s3",
            &[
                Field {
                    default: "https://s3.amazonaws.com",
                    ..field("endpoint")
                },
                field("region"),
                field("access_key_id"),
                secret("secret_access_key"),
            ],
        )
    },
    typed(
        "b2",
        "Backblaze B2",
        "b2",
        &[field("account"), secret("key")],
    ),
    typed(
        "webdav",
        "Nextcloud / WebDAV",
        "webdav",
        &[
            field("url"),
            Field {
                default: "nextcloud",
                ..field("vendor")
            },
            field("user"),
            secret("pass"),
        ],
    ),
    typed(
        "koofr",
        "Koofr",
        "koofr",
        &[field("user"), secret("password")],
    ),
    typed(
        "seafile",
        "Seafile",
        "seafile",
        &[field("url"), field("user"), secret("pass")],
    ),
    // rclone refuses to sign in without the region (`no region set`).
    Plugin {
        browser: true,
        ..typed(
            "zoho",
            "Zoho WorkDrive",
            "zoho",
            &[Field {
                default: "com",
                ..field("region")
            }],
        )
    },
    // Its login token is one of rclone's questions.
    typed("jottacloud", "Jottacloud", "jottacloud", &[]),
    // Newer than the rclone of Ubuntu 24.04 (1.60): `backends` tells (216c).
    typed(
        "mega",
        "MEGA",
        "mega",
        &[field("user"), secret("pass"), field("2fa")],
    ),
    Plugin {
        // rclone: its cache is never cleared on changes made elsewhere; off under a mount.
        preset: &[("enable_caching", "false")],
        ..typed(
            "protondrive",
            "Proton Drive",
            "protondrive",
            &[
                field("username"),
                secret("password"),
                field("2fa"),
                secret("mailbox_password"),
            ],
        )
    },
    // The 2FA code is one of rclone's questions (`config_2fa`).
    typed(
        "iclouddrive",
        "iCloud Drive",
        "iclouddrive",
        &[field("apple_id"), secret("password")],
    ),
];

/// The storage types of the installed rclone, from `rclone help backends` (two-space indented
/// `name   description` lines).
pub fn parse_backends(help: &str) -> Vec<String> {
    (help.lines())
        .take_while(|l| !l.starts_with("To see"))
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// The installed rclone's storage types; none without rclone or when it fails.
pub fn backends() -> Vec<String> {
    if !installed() {
        return Vec::new();
    }
    run(rclone(["help", "backends"]))
        .map(|s| parse_backends(&s))
        .unwrap_or_default()
}

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

/// Plugins whose rclone backend cannot rename (`mv` → EIO): Google Photos.
const NO_RENAME: &[&str] = &["gphotos"];

/// Whether a file can be renamed at `path`: not inside a mounted cloud of `NO_RENAME`.
pub fn renames(path: &Path) -> bool {
    root().is_none_or(|root| renames_under(&root, path))
}

fn renames_under(root: &Path, path: &Path) -> bool {
    let point = (path.strip_prefix(root).ok())
        .and_then(|rest| rest.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    !point.is_some_and(|n| of_point(&n).is_some_and(|(p, _)| NO_RENAME.contains(&p.id)))
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

/// Our own rclone (216d) or one in `PATH` (looked up, not run).
pub fn installed() -> bool {
    crate::rclone::own().is_some() || in_path("rclone")
}

fn in_path(program: &str) -> bool {
    crate::host::which(program).is_some()
}

fn rclone<const N: usize>(args: [&str; N]) -> Command {
    let mut c = crate::host::command(crate::rclone::program());
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
pub fn add(p: &Plugin, values: &[String], cancel: &Arc<AtomicBool>) -> Result<String, Error> {
    // rclone waits for the browser on this port; taken, it fails with its whole usage text.
    // Checked first, so no remote is written either.
    if std::net::TcpListener::bind(AUTH_ADDR).is_err() {
        return Err(Error::Failed(PORT_BUSY.into()));
    }
    let name = fresh_name(p, &names(&run(rclone(["listremotes", "--long"]))?));
    let mut c = rclone(["config", "create", "--", &name, p.kind]);
    c.args(options(p, values, obscure)?);
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

/// One of rclone's questions while creating an account (`--non-interactive` answers JSON).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// Given back with the answer (`--state`).
    pub state: String,
    /// The option asked (`config_login_token`).
    pub option: String,
    pub help: String,
    pub default: String,
    /// (value, help) to pick from.
    pub examples: Vec<(String, String)>,
    pub password: bool,
    /// Why the last answer was not taken.
    pub error: String,
    /// A page to open for the answer (the Yandex sign-in), else empty.
    pub open: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    Ask(Question),
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Answer {
    state: String,
    option: Option<AskedOption>,
    error: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AskedOption {
    name: String,
    help: String,
    #[serde(default)]
    default_str: String,
    #[serde(default)]
    examples: Vec<Example>,
    #[serde(default)]
    is_password: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Example {
    value: String,
    help: String,
}

/// rclone's `--non-interactive` answer: done, a question, or its error.
pub fn parse_step(json: &str) -> Result<Step, Error> {
    let a: Answer =
        serde_json::from_str(json).map_err(|e| Error::Failed(format!("rclone: {e}")))?;
    match a.option {
        Some(o) if !a.state.is_empty() => Ok(Step::Ask(Question {
            state: a.state,
            option: o.name,
            help: o.help,
            default: o.default_str,
            examples: (o.examples.into_iter())
                .map(|e| (e.value, e.help))
                .collect(),
            password: o.is_password,
            error: a.error,
            open: String::new(),
        })),
        _ if !a.error.is_empty() => Err(Error::Failed(a.error)),
        _ => Ok(Step::Done),
    }
}

/// Create an account of a plugin without a browser sign-in from its `fields` (`values` in their
/// order, empty ones left out): its name and rclone's first question, or `Done` once it lists.
pub fn create(p: &Plugin, values: &[String]) -> Result<(String, Step), Error> {
    let name = fresh_name(p, &names(&run(rclone(["listremotes", "--long"]))?));
    if p.id == "yandex" {
        let value = |i: usize| values.get(i).map_or("", |v| v.trim());
        return yandex_ask(value(0), value(1)).map(|q| (name, Step::Ask(q)));
    }
    let mut c = rclone([
        "config",
        "create",
        "--non-interactive",
        "--no-obscure",
        "--",
    ]);
    c.args([&name, p.kind]);
    let step = options(p, values, obscure).and_then(|args| {
        c.args(args);
        parse_step(&run(c).map_err(cut)?)
    });
    finish(&name, step).map(|s| (name, s))
}

/// Answer rclone's question about the account `name`: the next question, or `Done` once it lists.
pub fn answer(name: &str, q: &Question, result: &str) -> Result<Step, Error> {
    if let Some(app) = q.state.strip_prefix(YANDEX_STATE) {
        let (id, secret) = app.split_once('\n').unwrap_or((app, ""));
        // A wrong or expired code: asked again (the page is open already), nothing was created.
        let token = match yandex_token(id, secret, result.trim()) {
            Ok(t) => t,
            Err(Error::Failed(e)) => {
                return Ok(Step::Ask(Question {
                    error: e,
                    open: String::new(),
                    ..q.clone()
                }));
            }
            Err(e) => return Err(e),
        };
        return finish(name, yandex_create(name, id, secret, &token));
    }
    // Not obscured by rclone itself here (checked): a password would be written as it is.
    let step = (if q.password {
        obscure(result)
    } else {
        Ok(result.to_string())
    })
    .and_then(|result| {
        let mut c = rclone(["config", "update", "--non-interactive", "--no-obscure"]);
        c.args([
            "--continue",
            "--state",
            &q.state,
            "--result",
            &result,
            "--",
            name,
        ]);
        parse_step(&run(c).map_err(cut)?)
    });
    finish(name, step)
}

/// Our own question's state (never rclone's): the app's client ID and secret follow.
const YANDEX_STATE: &str = "shagoff-yandex\n";

/// The option of our question for the code Yandex shows after the sign-in.
pub const YANDEX_CODE: &str = "yandex_code";

/// The sign-in page of the user's Yandex app (an «API / debugging» one: its redirect is fixed to
/// `verification_code`, which shows the code to type, so rclone's own browser sign-in never ends).
pub fn yandex_url(client_id: &str) -> String {
    format!(
        "https://oauth.yandex.ru/authorize?response_type=code&client_id={}",
        url_encode(client_id)
    )
}

fn yandex_ask(id: &str, secret: &str) -> Result<Question, Error> {
    if id.is_empty() || secret.is_empty() {
        return Err(Error::Failed(crate::why::CLIENT_NEEDED.into()));
    }
    Ok(Question {
        state: format!("{YANDEX_STATE}{id}\n{secret}"),
        option: YANDEX_CODE.into(),
        help: "The code Yandex shows after you allow the access".into(),
        default: String::new(),
        examples: Vec::new(),
        password: false,
        error: String::new(),
        open: yandex_url(id),
    })
}

/// The code for a token as rclone keeps it (`oauth.yandex.ru/token`, the form through stdin: the
/// secret stays off argv). rclone refreshes it itself afterwards.
fn yandex_token(id: &str, secret: &str, code: &str) -> Result<String, Error> {
    use std::io::Write;
    use std::process::Stdio;
    let failed = |e: std::io::Error| Error::Failed(format!("curl: {e}"));
    let mut child = (crate::host::command("curl"))
        .args([
            "-sS",
            "-m",
            "60",
            "--data",
            "@-",
            "https://oauth.yandex.ru/token",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", id),
        ("client_secret", secret),
    ]);
    let wrote = (child.stdin.take().expect("piped")).write_all(body.as_bytes());
    let out = child.wait_with_output().map_err(failed)?;
    wrote.map_err(failed)?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Failed(e.trim().into()));
    }
    rclone_token(
        &String::from_utf8_lossy(&out.stdout),
        jiff::Timestamp::now(),
    )
}

/// The account with the app and its token.
fn yandex_create(name: &str, id: &str, secret: &str, token: &str) -> Result<Step, Error> {
    // ponytail: secret and token on argv while rclone writes them (a moment, no network);
    // writing rclone.conf ourselves if it ever matters.
    let mut c = rclone(["config", "create", "--non-interactive", "--no-obscure"]);
    c.args([
        "--",
        name,
        "yandex",
        "client_id",
        id,
        "client_secret",
        secret,
    ]);
    // A token given: no "replace it?" question.
    c.args(["token", token, "config_refresh_token", "false"]);
    parse_step(&run(c).map_err(cut)?)
}

#[derive(serde::Deserialize)]
struct Token {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error_description: Option<String>,
    error: Option<String>,
}

/// Yandex's token answer as rclone keeps a token, or its error.
fn rclone_token(answer: &str, now: jiff::Timestamp) -> Result<String, Error> {
    let t: Token = serde_json::from_str(answer)
        .map_err(|_| Error::Failed(format!("oauth.yandex.ru: {}", answer.trim())))?;
    let Some(access) = t.access_token else {
        let e = t.error_description.or(t.error).unwrap_or_default();
        return Err(Error::Failed(format!("oauth.yandex.ru: {e}")));
    };
    let life = jiff::SignedDuration::from_secs(t.expires_in.unwrap_or(3600));
    Ok(serde_json::json!({
        "access_token": access,
        "token_type": "OAuth",
        "refresh_token": t.refresh_token.unwrap_or_default(),
        "expiry": (now + life).to_string(),
    })
    .to_string())
}

fn url_encode(s: &str) -> String {
    (s.bytes())
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

/// `application/x-www-form-urlencoded`.
fn form(pairs: &[(&str, &str)]) -> String {
    (pairs.iter())
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `preset` and the typed `values` as `key value` arguments, secrets through `hide`.
fn options(
    p: &Plugin,
    values: &[String],
    hide: impl Fn(&str) -> Result<String, Error>,
) -> Result<Vec<String>, Error> {
    let mut args = Vec::new();
    for (k, v) in p.preset {
        args.extend([k.to_string(), v.to_string()]);
    }
    for (f, v) in p.fields.iter().zip(values) {
        let v = match if f.secret { v.as_str() } else { v.trim() } {
            // Cleared: the field's default (Zoho has no sign-in without its region).
            "" => f.default,
            v => v,
        };
        if v.is_empty() {
            continue;
        }
        args.extend([
            f.key.to_string(),
            if f.secret { hide(v)? } else { v.into() },
        ]);
    }
    Ok(args)
}

fn cut(e: Error) -> Error {
    match e {
        Error::Failed(text) => Error::Failed(rclone_error(&text)),
        e => e,
    }
}

/// Done: the account must list (rclone writes any password without trying it), else it goes.
fn finish(name: &str, step: Result<Step, Error>) -> Result<Step, Error> {
    let step = step.and_then(|s| {
        if s == Step::Done {
            let target = format!("{name}:");
            let mut c = rclone(["lsd", "--contimeout", "15s", "--timeout", "30s"]);
            c.args(["--retries", "1", "--low-level-retries", "1", "--", &target]);
            match run(c) {
                // S3 keys for one bucket may not list them all: signed in all the same.
                Err(Error::Failed(e)) if e.contains("AccessDenied") => {}
                r => drop(r.map_err(cut)?),
            }
        }
        Ok(s)
    });
    if step.is_err() {
        let _ = forget(name);
    }
    step
}

/// rclone's reversible encoding of a password, as `rclone.conf` keeps it. The plain one goes in
/// through stdin; the encoded one is on `config create`'s command line while that runs (no network
/// for the typed plugins: a moment; the `lsd` check is a process of its own, without it).
// ponytail: reversible on argv for that moment; writing rclone.conf ourselves if it ever matters.
fn obscure(password: &str) -> Result<String, Error> {
    use std::io::Write;
    use std::process::Stdio;
    let failed = |e: std::io::Error| Error::Failed(e.to_string());
    let mut child = rclone(["obscure", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    // No newline: rclone keeps it as part of the password.
    let wrote = (child.stdin.take().expect("piped")).write_all(password.as_bytes());
    let out = child.wait_with_output().map_err(failed)?;
    wrote.map_err(failed)?;
    if !out.status.success() {
        return Err(Error::Failed(
            String::from_utf8_lossy(&out.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
        let mounts = crate::host::mounts();
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
    let mounts = crate::host::mounts();
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
    let mut c = crate::host::command(bin);
    c.arg(if lazy { "-uz" } else { "-u" }).arg(point);
    run(c).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REMOTES: &str = "gdrive:   drive\nmy drive: drive\nnas:      sftp\nonedrive: onedrive\n";

    #[test]
    fn yandex_code_becomes_an_rclone_token() {
        let now: jiff::Timestamp = "2026-11-13T21:00:00Z".parse().unwrap();
        let t = rclone_token(
            r#"{"access_token":"y0_a","expires_in":31536000,"refresh_token":"1:r","token_type":"bearer"}"#,
            now,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["access_token"], "y0_a");
        assert_eq!(v["refresh_token"], "1:r");
        assert_eq!(v["token_type"], "OAuth");
        assert_eq!(v["expiry"], "2027-11-13T21:00:00Z");
        let e = rclone_token(
            r#"{"error":"bad_verification_code","error_description":"Code has expired"}"#,
            now,
        );
        assert_eq!(
            e,
            Err(Error::Failed("oauth.yandex.ru: Code has expired".into()))
        );
    }

    #[test]
    fn yandex_asks_for_the_code_of_the_users_own_app() {
        assert_eq!(
            yandex_ask("", "s"),
            Err(Error::Failed(crate::why::CLIENT_NEEDED.into()))
        );
        let q = yandex_ask("ab12", "s3").unwrap();
        assert_eq!(q.option, YANDEX_CODE);
        assert_eq!(
            q.open,
            "https://oauth.yandex.ru/authorize?response_type=code&client_id=ab12"
        );
        assert_eq!(q.state.strip_prefix(YANDEX_STATE), Some("ab12\ns3"));
        assert_eq!(form(&[("a b", "x&y"), ("c", "d")]), "a%20b=x%26y&c=d");
    }

    #[test]
    fn regression_google_photos_is_written_without_rename() {
        let root = Path::new("/run/user/1/shagoff-clouds");
        assert!(!renames_under(
            root,
            &root.join("gphotos:gphotos/upload/a.jpg")
        ));
        assert!(!renames_under(root, &root.join("gphotos:my photos/upload")));
        assert!(renames_under(root, &root.join("gdrive:gdrive/a.jpg")));
        assert!(renames_under(
            root,
            Path::new("/home/u/gphotos:gphotos/a.jpg")
        ));
        assert!(renames_under(root, &root.join("junk")));
    }

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
    fn parse_remotes_knows_the_browser_sign_in_clouds() {
        let r = parse_remotes("a: yandex\nb: pcloud\nc: gphotos\nd: hidrive\ne: sftp\n");
        let got: Vec<_> = r.iter().map(|r| r.plugin.id).collect();
        assert_eq!(got, ["yandex", "pcloud", "gphotos", "hidrive"]);
        let plugin = PLUGINS.iter().find(|p| p.id == "gphotos").unwrap();
        assert_eq!(fresh_name(plugin, &["gphotos".to_string()]), "gphotos-2");
    }

    // As rclone 1.60.1 answers `config create t3 jottacloud --non-interactive` (cut down).
    const QUESTION: &str = r#"{
	"State": "auth_type_done",
	"Option": {
		"Name": "config_type",
		"Help": "Select authentication type.",
		"Default": "standard",
		"Value": null,
		"Examples": [
			{"Value": "standard", "Help": "Standard authentication.", "Provider": ""},
			{"Value": "legacy", "Help": "Legacy authentication.", "Provider": ""}
		],
		"IsPassword": false,
		"DefaultStr": "standard",
		"Type": "string"
	},
	"Error": "",
	"Result": ""
}"#;

    #[test]
    fn parse_step_reads_rclone_answers() {
        let done = r#"{"State": "", "Option": null, "Error": "", "Result": ""}"#;
        assert_eq!(parse_step(done), Ok(Step::Done));
        let Ok(Step::Ask(q)) = parse_step(QUESTION) else {
            panic!("a question")
        };
        assert_eq!(
            (q.state.as_str(), q.option.as_str()),
            ("auth_type_done", "config_type")
        );
        assert_eq!(q.default, "standard");
        assert_eq!(q.examples[1].0, "legacy");
        assert!(!q.password);
        let failed = r#"{"State": "", "Option": null, "Error": "bad token", "Result": ""}"#;
        assert_eq!(parse_step(failed), Err(Error::Failed("bad token".into())));
        assert!(parse_step("Usage: rclone").is_err());
    }

    #[test]
    fn options_skip_empty_fields_and_hide_secrets() {
        let s3 = PLUGINS.iter().find(|p| p.id == "s3").unwrap();
        let values = ["https://r2.example", " ", "AK", "p w"].map(String::from);
        let args = options(s3, &values, |v| Ok(format!("hidden({v})"))).unwrap();
        assert_eq!(
            args,
            [
                "provider",
                "Other",
                "endpoint",
                "https://r2.example",
                "access_key_id",
                "AK",
                "secret_access_key",
                "hidden(p w)"
            ]
        );
    }

    #[test]
    fn parse_backends_reads_rclone_help() {
        // rclone 1.60.1, cut down.
        let help = "All rclone backends:\n\n  alias        Alias for an existing remote\n  \
                    yandex       Yandex Disk\n  zoho         Zoho\n\n\
                    To see more info about a particular backend use:\n  rclone help backend <name>\n";
        assert_eq!(parse_backends(help), ["alias", "yandex", "zoho"]);
    }

    #[test]
    fn a_cleared_field_takes_its_default() {
        let zoho = PLUGINS.iter().find(|p| p.id == "zoho").unwrap();
        let args = options(zoho, &[" ".into()], |v| Ok(v.into())).unwrap();
        assert_eq!(args, ["region", "com"]);
    }

    #[test]
    fn plugins_have_unique_ids_and_kinds() {
        for (i, p) in PLUGINS.iter().enumerate() {
            let later = &PLUGINS[i + 1..];
            assert!(
                !later.iter().any(|q| q.id == p.id || q.kind == p.kind),
                "{}",
                p.id
            );
            assert!(
                p.browser || p.id == "jottacloud" || !p.fields.is_empty(),
                "{}",
                p.id
            );
        }
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
