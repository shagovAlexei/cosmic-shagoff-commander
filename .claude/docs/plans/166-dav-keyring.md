# 166. WebDAV password in the keyring — plan

Spec: `.claude/docs/specs/166-dav-keyring.md`.

1. core `keyring`: `gvfs_attrs(url, realm)` with dav / davs; `realm(headers)`; `fetch_realm` (curl);
   `label` with `/object`; `keeps` for dav. Tests.
2. Live check: docker WebDAV, save through the app, `gio mount` without a password.
3. Review: realm via OPTIONS then PROPFIND, -L, token / Digest / boundaries, `KeepError`;
   `mount::connect` waits for the local path (found live).
4. CLAUDE.md, tc-reference, ROADMAP, TESTING, version 0.80.0.
