# 164. Browse the network: inside the Windows network

ROADMAP «Сеть, дальше: заход внутрь `smb:///` при обзоре»; spec 26 browsed one level only.

- Ctrl+F → «Обзор сети» lists `network:///` as before. A found entry that is an SMB place above a
  share — `smb:///` (Windows network), `smb://WORKGROUP/`, `smb://server/` (`mount::deeper`) —
  gets a «›» button next to it: it lists that place in its turn (workgroups → servers →
  shares). A share (`smb://server/share/`) only fills the address, as every entry does.
- gvfs lists `smb:` places only once mounted («The specified location is not mounted»):
  `mount::browse(at, password, cancel)` mounts `at` then (`gio mount`, the dialog's password
  answers a prompt, as for connecting) and lists again.
- The place being listed is shown above the found entries (none for `network:///`). «Обзор
  сети» again starts from `network:///`; pressing it while looking stops as before (122).
- A result that is not for the place asked last is dropped (`Message::Browsed(at, …)`), so a
  stopped listing finishing late never shows the wrong level.
- Not done: a «back» button (the button starts over), WebDAV's password (gvfs keys it by the
  server's realm).
