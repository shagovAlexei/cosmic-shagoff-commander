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
