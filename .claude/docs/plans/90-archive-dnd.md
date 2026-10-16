# 90. Archive drag and drop — plan

Spec: `../specs/90-archive-dnd.md`.

1. Core: `clipboard::INNER`, decoded as a uri-list copy (test).
2. `clip::drag(.., inner, ..)`: `Files(mime, inner)` offers only `INNER`; `Paste` accepts it.
3. `view::dnd_row`: archive rows are sources (inner); archive file rows are drop zones.
4. App test: drop out of an archive → Extract; onto an archive pane / row → Repack.
