# 99. Find: text inside archives

ROADMAP «Поиск, дальше: текст внутри архивов». With «Искать в архивах» on, Alt+F7 matched members
by name only; a text (or regex) skipped archives.

- `archive::for_each_file`: every file member streamed with its path, size and mtime (zip — a
  password already given is used, a member that can't be read is passed over; 7z; tar.* as one
  stream). Stop ends it.
- `search`: the text / regex readers take any `Read`; in archives members pass the same name /
  hidden / size / date filters, then their contents are searched.
- The checkbox reads «(имена и текст)».
