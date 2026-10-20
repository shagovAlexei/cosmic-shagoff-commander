# 164. Browse the network: inside the Windows network

ROADMAP «Сеть, дальше: заход внутрь `smb:///` при обзоре»; spec 26 browsed one level only.

- Ctrl+F → «Обзор сети» lists `network:///` as before. A found entry that is an SMB place above a
  share — `smb:///` (Windows network), `smb://WORKGROUP/`, `smb://server/` (`mount::deeper`) —
  gets a «›» button next to it: it lists that place in its turn (workgroups → servers →
  shares). A share (`smb://server/share/`) only fills the address, as every entry does.
- gvfs lists `smb:` places only once mounted («The specified location is not mounted»):
  `mount::browse(at, password, cancel)` mounts a `deeper` place first (`gio mount`; «already
  mounted» is fine, the listing decides, as in `connect`), then lists it. The dialog's password
  answers its prompt only when the address field names the same host (`mount::host`): a
  password typed for one server never goes to another.
- The place being listed is shown above the found entries (none for `network:///`). «Обзор
  сети» again starts from `network:///`; pressing it while looking stops as before (122).
- Starting a browse clears what was found. A result is taken only from the browse under way
  (`Message::Browsed(stop, …)`, its stop flag): a stopped or replaced one finishing late is
  dropped and never takes the newer one's stop button.
- Mounts made for browsing stay until logout (as in «Files»).
- Not done: a «back» button (the button starts over), WebDAV's password (gvfs keys it by the
  server's realm).
