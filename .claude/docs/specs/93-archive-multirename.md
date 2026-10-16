# 93. Multi-rename in archives

ROADMAP «Архив, дальше: Ctrl+M в архиве».

Ctrl+M inside an archive said «read-only». Shift+F6 there already repacks with `Change::Rename`.

- `Change::Rename` takes a list of (from, to) pairs, done in order in the staging dir: one rewrite
  for the whole Ctrl+M, swaps go through `multirename::plan`'s parked names as on disk.
- Ctrl+M's OK inside an archive: the plan's pairs made relative to the archive → `Job::Repack`.
- A failed rename (answered Skip) leaves the archive unchanged (as Shift+F6).
