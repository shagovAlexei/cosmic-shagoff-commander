# 217. Find duplicate files (Alt+F7)

TC: Alt+F7 → «Advanced» → «Find duplicate files» with «same name», «same size», «same contents»; the
result list shows the groups one after another, divided by a line.

## Behaviour

- In the find dialog, next to «Search in archives», a checkbox «Find duplicates». On, a row under it:
  «Same name», «Same size», «Same contents» (defaults: off, on, on). None ticked → the search does
  not start: «Duplicates: tick what to compare».
- The walk is the usual one (mask, text, size, date, attributes all still filter); only regular files
  on disk count (no dirs, no symlinks, no archive members: the latter have no content to hash cheaply).
- After the walk the files are grouped: by name (if ticked) and size (if «size» or «contents»), then
  every group of 2+ by SHA-256 of the contents (if ticked). Groups of one are dropped. Groups come in
  the order of their first file; the files in walk order.
- While hashing, the progress line shows the file being read; Stop stops it (no groups then).
- The list shows the groups with a divider between them; Enter / «Go to file» as before; «To panel»
  lists all the files of all groups.
- The four boxes are remembered with the other find options (`FindPrefs`).

## Not done

- Archive members (TC compares them only by name / size; ours would need the index sizes — add on demand).
- Partial hashing (first block first) — a full hash of each same-size file; fine for a home dir.
