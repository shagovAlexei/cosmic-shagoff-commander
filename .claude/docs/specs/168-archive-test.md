# 168. Test archives (Alt+Shift+F9)

TC «Test archives» (Alt+Shift+F9), missing until now.

- The archives among the targets (by name: zip, tar.*, 7z, and single-file .gz / .bz2 / .xz /
  .zst; dirs by the panel's rows, nothing inside an archive) are read through in the background:
  `archive::test` streams every member to its end, so zip / gzip / 7z check their CRCs, and a
  member that cannot be opened (damage, a wrong or missing password) is an error, not skipped.
  The search's `for_each_file` shares the walk (`archive::files`) in its lenient mode.
- Result in the status line: all fine — how many archives and files read; else «N of M damaged»
  and a line per bad archive with its error. Leaving the dir stops it (the tab's `counting`).
- No targets that are archives: an error saying so.
