//! Network passwords in the Secret Service keyring (gnome-keyring, KWallet…), saved the way gvfs
//! saves them (`gvfskeyring.c`, schema `org.gnome.keyring.NetworkPassword`): sftp, smb, ftp and
//! WebDAV look there before asking, so a saved password is not asked for again — by us, nor by any other
//! gvfs program. Over D-Bus (zbus), no libsecret / glib in the build.

use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// The attributes gvfs looks the password for `url` up by (its `build_network_attributes`), `None`
/// for a scheme it does not keep. WebDAV (`gvfsbackenddav.c`): protocol `http` for davs too,
/// authtype `basic` whatever the server asks, the server's `realm` as object (`None`: not known,
/// not kept) and the port of the request, the default filled in. Others: the port as typed (ftp:
/// not the default 21, as gvfs).
pub fn gvfs_attrs(url: &str, realm: Option<&str>) -> Option<Vec<(&'static str, String)>> {
    let u = url::Url::parse(url.trim()).ok()?;
    let dav = matches!(u.scheme(), "dav" | "davs");
    let protocol = match u.scheme() {
        p @ ("sftp" | "smb" | "ftp") => p,
        _ if dav => "http",
        _ => return None,
    };
    let user = percent_decode_str(u.username()).decode_utf8().ok()?;
    // smb: `DOMAIN;user@server`.
    let (domain, user) = match user.split_once(';') {
        Some((d, u)) if protocol == "smb" => (Some(d.to_string()), u.to_string()),
        _ => (None, user.into_owned()),
    };
    let mut attrs = Vec::new();
    if !user.is_empty() {
        attrs.push(("user", user));
    }
    attrs.push(("server", u.host_str()?.to_string()));
    attrs.extend(domain.map(|d| ("domain", d)));
    attrs.push(("protocol", protocol.to_string()));
    if protocol == "sftp" {
        attrs.push(("authtype", "password".to_string()));
    }
    if dav {
        attrs.push(("object", realm?.to_string()));
        attrs.push(("authtype", "basic".to_string()));
        let default = if u.scheme() == "davs" { 443 } else { 80 };
        attrs.push(("port", u.port().unwrap_or(default).to_string()));
        return Some(attrs);
    }
    attrs.extend(
        u.port()
            .filter(|&p| p != 0)
            .map(|p| ("port", p.to_string())),
    );
    Some(attrs)
}

/// Whether `url`'s scheme is one whose password can be kept (the host may be still being typed).
pub fn keeps(url: &str) -> bool {
    url.trim()
        .split_once("://")
        .is_some_and(|(s, _)| matches!(s, "sftp" | "smb" | "ftp" | "dav" | "davs"))
}

/// The item's name, as gvfs names it: `user@server:port/object`.
fn label(attrs: &[(&str, String)]) -> String {
    let get = |k| attrs.iter().find(|(a, _)| *a == k).map(|(_, v)| v.as_str());
    let mut s = get("user").map(|u| format!("{u}@")).unwrap_or_default();
    s += get("server").unwrap_or_default();
    if let Some(p) = get("port") {
        s += &format!(":{p}");
    }
    if let Some(o) = get("object") {
        s += &format!("/{o}");
    }
    s
}

/// The realm of the first `WWW-Authenticate` header that names one (`Basic realm="…"`).
pub fn realm(headers: &str) -> Option<String> {
    headers.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("www-authenticate") {
            return None;
        }
        let at = value.to_ascii_lowercase().find("realm=\"")? + "realm=\"".len();
        let (mut out, mut escaped) = (String::new(), false);
        for c in value[at..].chars() {
            match c {
                _ if escaped => {
                    out.push(c);
                    escaped = false;
                }
                '\\' => escaped = true,
                '"' => return Some(out),
                _ => out.push(c),
            }
        }
        None
    })
}

/// Ask a WebDAV server its realm (gvfs keys the password by it): `curl` OPTIONS, as gvfs mounts,
/// without the user (no credentials sent), 10 s. Blocking.
fn fetch_realm(url: &str) -> Option<String> {
    let mut u = url::Url::parse(url.trim()).ok()?;
    let http = if u.scheme() == "davs" {
        "https"
    } else {
        "http"
    };
    u.set_username("").ok()?;
    u.set_password(None).ok()?;
    let rest = u.as_str().split_once("://")?.1.to_string();
    let out = std::process::Command::new("curl")
        .args([
            "-sk",
            "-m",
            "10",
            "-X",
            "OPTIONS",
            "-o",
            "/dev/null",
            "-D",
            "-",
        ])
        .arg(format!("{http}://{rest}"))
        .output()
        .ok()?;
    realm(&String::from_utf8_lossy(&out.stdout))
}

/// Keep `password` for `url` in the default keyring (replacing one kept before). Blocking: the
/// keyring may ask to be unlocked first.
pub fn save(url: &str, password: &str) -> Result<(), String> {
    let dav = url.trim().starts_with("dav");
    let realm = if dav { fetch_realm(url) } else { None };
    if dav && realm.is_none() {
        return Err(format!("{url}: the server names no realm"));
    }
    let attrs = gvfs_attrs(url, realm.as_deref()).ok_or_else(|| url.to_string())?;
    store(&label(&attrs), &attrs, password).map_err(|e| e.to_string())
}

const DEST: &str = "org.freedesktop.secrets";

fn store(label: &str, attrs: &[(&str, String)], password: &str) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let proxy = |path: &str, iface: &'static str| Proxy::new(&conn, DEST, path.to_string(), iface);
    let service = proxy("/org/freedesktop/secrets", "org.freedesktop.Secret.Service")?;
    // "plain": the secret crosses the session bus as is, as with `secret-tool` without libgcrypt.
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service.call("OpenSession", &("plain", Value::from("")))?;
    let collection: OwnedObjectPath = service.call("ReadAlias", &("default",))?;
    if collection.as_str() == "/" {
        return Err(zbus::Error::Failure("no default keyring".into()));
    }
    let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service.call("Unlock", &(vec![&collection],))?;
    answer(&conn, &prompt)?;
    let mut map: HashMap<&str, &str> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    map.insert("xdg:schema", "org.gnome.keyring.NetworkPassword");
    let props = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from(label)),
        ("org.freedesktop.Secret.Item.Attributes", Value::from(map)),
    ]);
    let secret = (
        &session,
        Vec::<u8>::new(),
        password.as_bytes(),
        "text/plain",
    );
    let coll = proxy(collection.as_str(), "org.freedesktop.Secret.Collection")?;
    let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
        coll.call("CreateItem", &(props, secret, true))?;
    answer(&conn, &prompt)?;
    let _: () = proxy(session.as_str(), "org.freedesktop.Secret.Session")?.call("Close", &())?;
    Ok(())
}

/// A keyring prompt (unlock): shown, and waited for. `/`: none needed.
fn answer(conn: &Connection, prompt: &OwnedObjectPath) -> zbus::Result<()> {
    if prompt.as_str() == "/" {
        return Ok(());
    }
    let p = Proxy::new(conn, DEST, prompt.as_str(), "org.freedesktop.Secret.Prompt")?;
    let mut done = p.receive_signal("Completed")?;
    let _: () = p.call("Prompt", &("",))?;
    let msg = done
        .next()
        .ok_or_else(|| zbus::Error::Failure("no answer".into()))?;
    let (dismissed, _): (bool, OwnedValue) = msg.body().deserialize()?;
    match dismissed {
        true => Err(zbus::Error::Failure("dismissed".into())),
        false => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(url: &str) -> Vec<(&'static str, String)> {
        gvfs_attrs(url, None).unwrap()
    }

    fn v(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn sftp_as_gvfs_looks_it_up() {
        assert_eq!(
            a("sftp://bob@host.lan/home"),
            v(&[
                ("user", "bob"),
                ("server", "host.lan"),
                ("protocol", "sftp"),
                ("authtype", "password")
            ])
        );
        // No user: none (gvfs looks up without one, any user's matches); a port as typed.
        assert_eq!(
            a("sftp://host:2222"),
            v(&[
                ("server", "host"),
                ("protocol", "sftp"),
                ("authtype", "password"),
                ("port", "2222")
            ])
        );
    }

    #[test]
    fn smb_domain_and_ftp_default_port() {
        assert_eq!(
            a("smb://WORK;ann@nas/share"),
            v(&[
                ("user", "ann"),
                ("server", "nas"),
                ("domain", "WORK"),
                ("protocol", "smb")
            ])
        );
        assert_eq!(
            a("ftp://joe%20x@ftp.example.org:21/pub"),
            v(&[
                ("user", "joe x"),
                ("server", "ftp.example.org"),
                ("protocol", "ftp")
            ])
        );
        assert_eq!(a("ftp://h:2121")[2], ("port", "2121".to_string()));
    }

    #[test]
    fn webdav_with_its_realm_the_rest_not_kept() {
        // gvfsbackenddav.c: protocol "http" (davs too), authtype "basic", the realm as object,
        // the port as the request has it (the default filled in).
        assert_eq!(
            gvfs_attrs("dav://tester@localhost:8088/", Some("WebDAV")).unwrap(),
            v(&[
                ("user", "tester"),
                ("server", "localhost"),
                ("protocol", "http"),
                ("object", "WebDAV"),
                ("authtype", "basic"),
                ("port", "8088")
            ])
        );
        let davs = gvfs_attrs("davs://u@cloud/remote.php", Some("Nextcloud")).unwrap();
        assert_eq!(davs.last(), Some(&("port", "443".to_string())));
        assert_eq!(
            gvfs_attrs("dav://h/", Some("r")).unwrap().last().unwrap().1,
            "80"
        );
        assert_eq!(gvfs_attrs("davs://u@cloud/remote.php", None), None); // realm unknown
        assert_eq!(gvfs_attrs("nfs://h/x", None), None);
        assert_eq!(gvfs_attrs("not a url", None), None);
        assert!(keeps("sftp://") && keeps(" smb://n") && keeps("davs://x") && !keeps("nas"));
    }

    #[test]
    fn realm_from_the_answer() {
        let headers = "HTTP/1.1 401 Unauthorized\r\nServer: x\r\n\
                       www-authenticate: Basic realm=\"My \\\"Cloud\\\"\", charset=\"UTF-8\"\r\n";
        assert_eq!(realm(headers).as_deref(), Some("My \"Cloud\""));
        let digest = "WWW-Authenticate: Digest qop=\"auth\", realm=\"dav\", nonce=\"1\"\n";
        assert_eq!(realm(digest).as_deref(), Some("dav"));
        assert_eq!(realm("HTTP/1.1 200 OK\r\n"), None);
    }

    #[test]
    fn labels_as_gvfs() {
        assert_eq!(label(&a("sftp://bob@h:22")), "bob@h:22");
        assert_eq!(label(&a("smb://nas/s")), "nas");
        let dav = gvfs_attrs("dav://t@h:8088/", Some("WebDAV")).unwrap();
        assert_eq!(label(&dav), "t@h:8088/WebDAV");
    }
}
