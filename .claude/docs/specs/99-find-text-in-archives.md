# 99. Find: text inside archives

ROADMAP «Поиск, дальше: текст внутри архивов». With «Искать в архивах» on, Alt+F7 matched members
by name only; a text (or regex) skipped archives.

- `archive::for_each_file`: every file member streamed with its path, size and mtime (zip — a
  password already given is used, a member that can't be read is passed over; 7z; tar.* as one
  stream). Stop ends it.
- `search`: the text / regex readers take any `Read`; in archives members pass the same name /
  hidden / size / date filters, then their contents are searched.
- Member names pass `safe_path` (`./a` → `a`; absolute / `..` ones dropped: a result must stay
  inside the archive). 7z: what a member's reader left unread is drained (a solid block is one
  stream). Links (tar hard links too) are not searched.
- The checkbox reads «(имена и текст)».
