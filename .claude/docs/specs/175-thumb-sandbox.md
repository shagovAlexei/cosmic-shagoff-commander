# 175. Thumbnailers in a bwrap sandbox

Follows 174 («Not sandboxed … upgrade path: bwrap»).

- When `bwrap` is installed and may make its sandbox (probed once: `bwrap … true`; unprivileged
  user namespaces can be off), a thumbnailer runs as gnome-desktop runs it: `/usr` and `/etc`
  read-only, `/lib*` `/bin` `/sbin` as links into `/usr` (or bound when not merged), `--proc`,
  `--dev`, `--unshare-all` (no network, no other process), `--die-with-parent`, `--new-session`.
- Inside, the file is `/tmp/gnome-desktop-file-to-thumbnail.<ext>` (bound read-only) and the output
  `/tmp/gnome-desktop-thumbnailer.png`; `/tmp` is our own empty dir (0700 in `$TMPDIR`), removed
  after. GNOME's names: Ubuntu's AppArmor profile of evince-thumbnailer allows them.
- The home and every other file are not there: a crafted PDF / font / video meets a parser that
  can read and send nothing.
- Without a working bwrap: as in 174 (unsandboxed, as cosmic-files).
- Checked: evince (PDF), cosmic-player (video), gdk-pixbuf (SVG) all work inside.
