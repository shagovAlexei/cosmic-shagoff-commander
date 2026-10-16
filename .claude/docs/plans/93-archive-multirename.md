# 93. Multi-rename in archives — plan

Spec: `../specs/93-archive-multirename.md`.

1. Core: `Change::Rename(Vec<(from, to)>)` (test: a ↔ b swap through a parked name).
2. App: Ctrl+M not read-only; its OK in an archive → `Job::Repack { Rename(pairs) }` (test).
