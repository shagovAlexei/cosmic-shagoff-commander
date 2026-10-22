# 174. Thumbnails of PDF, video, SVG — the desktop's thumbnailers

ROADMAP «Вид, дальше: эскизы … видео / PDF — нет».

- A file we don't decode ourselves (not a png / jpg / gif / bmp / webp / ico ≤ 64 MB) gets its
  picture from the thumbnailer the desktop installed for its type (freedesktop thumbnailer spec:
  `thumbnailers/*.thumbnailer` in the XDG data dirs — evince for PDF, cosmic-player for video,
  librsvg / gdk-pixbuf for SVG, fonts, epub…). No new dependency.
- The type comes from the name, by shared-mime-info's `mime/globs2` (plain `*.suffix` globs, the
  longest suffix first). No magic sniffing, no aliases / subclasses (ponytail).
- A thumbnailer whose `TryExec` (else its program) isn't installed is left out; an earlier data
  dir's file of a name hides later ones.
- `Exec` codes: `%i` path, `%u` URI, `%o` output, `%s` 128, `%%`; paths stay bytes.
- Output: `/tmp/.gnome_desktop_thumbnail.shagoff.<pid>.<n>.png` — GNOME's prefix in /tmp itself,
  since Ubuntu's AppArmor profile lets evince-thumbnailer write only there; created first with
  O_EXCL; removed after. 10 s timeout, then killed. A failure is remembered (same mtime): no rerun.
- Same `BUSY_MAX` (4 at once), same in-memory cache.
- On timeout the thumbnailer's whole process group is killed (a worker must not write the output
  after it is removed): `thumbnailer::run`.
- A picture we'd decode but over 64 MB gets its icon, never a thumbnailer (the cap holds).
- `globs2`: the `cs` flag kept (`*.C` vs `*.c`), the heaviest weight across all dirs; an override
  that isn't installed hides nothing.
- Not sandboxed (GNOME runs thumbnailers under bwrap; cosmic-files doesn't): only in the
  Thumbnails view, which the user turns on. ~~Upgrade path: a `bwrap` prefix when present.~~ 175.
- Not done: a failure is forgotten when the whole cache is dropped (`CACHE_MAX`).
