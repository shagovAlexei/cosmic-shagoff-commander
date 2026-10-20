# 166. WebDAV password in the keyring

ROADMAP «Сеть, дальше: пароль WebDAV (gvfs ищет его по realm сервера)»; 150 kept sftp / smb / ftp.

gvfs 1.54 (`gvfsbackenddav.c`, `soup_authenticate` / `keyring_save_authinfo`) looks a dav / davs
password up by: `user`, `server`, protocol `http` (davs too), object = the realm of the server's
`WWW-Authenticate`, authtype `basic` (whatever the server asks), port of the request — always
there, the default 80 / 443 filled in (checked live: an item without the port, or with -1, is not
found; with 80 for `dav://host/` it is).

- Ctrl+F: «Remember the password» shows for `dav://` / `davs://` too (`keyring::keeps`).
- Once connected, `keyring::save` asks the server its realm: `curl -sk -X OPTIONS` (as gvfs mounts)
  without the user, 10 s; `keyring::realm` parses the header (quoted, escapes). No realm (no
  password on the server, curl failed): not kept, the error says so; the connection stays.
- The item's label as gvfs's: `user@server:port/realm`.
- Checked end to end: saved by the program, `gio mount` of the same address no longer asks.
