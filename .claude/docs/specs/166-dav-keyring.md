# 166. WebDAV password in the keyring

ROADMAP «Сеть, дальше: пароль WebDAV (gvfs ищет его по realm сервера)»; 150 kept sftp / smb / ftp.

gvfs 1.54 (`gvfsbackenddav.c`, `soup_authenticate` / `keyring_save_authinfo`) looks a dav / davs
password up by: `user`, `server`, protocol `http` (davs too), object = the realm of the server's
`WWW-Authenticate`, authtype `basic` (whatever the server asks), port of the request — always
there, the default 80 / 443 filled in (checked live: an item without the port, or with -1, is not
found; with 80 for `dav://host/` it is).

- Ctrl+F: «Remember the password» shows for `dav://` / `davs://` too (`keyring::keeps`).
- Once connected, `keyring::save` asks the server its realm as gvfs mounts: `curl` OPTIONS, then
  PROPFIND (Depth 0) if OPTIONS is let through without a password; redirects followed, no user
  sent, 10 s each. `keyring::realm` reads the auth params in order (quoted with escapes or a bare
  token; `realm=` inside another parameter is not it) and prefers a Digest challenge's realm, as
  libsoup does. No realm: «the server asks for no password» (`KeepError::NoRealm`, en / ru); a
  curl failure: its own message. Errors never carry the address (a password may be typed in it).
  The connection stays either way.
- Found on the way (26): `gio mount` returns before gvfsd-fuse has the mount, so the `gio info`
  right after it had no local path («no local path (gvfs-fuse not running?)», WebDAV on localhost
  every time). `mount::connect` now asks again for up to 2 s once mounted.
- The item's label as gvfs's: `user@server:port/realm`.
- Checked end to end: saved by the program, `gio mount` of the same address no longer asks.
