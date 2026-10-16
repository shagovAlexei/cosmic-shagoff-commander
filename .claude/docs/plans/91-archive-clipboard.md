# 91. Clipboard in archives — plan

Spec: `../specs/91-archive-clipboard.md`.

1. Core: `INNER` decoded as the gnome format (copy / cut header) (test).
2. `clip::put(kind, paths, inner)`; `Files` gives `INNER` the gnome payload.
3. `App`: drop the Ctrl+C-extracts arm and `ClipCut` / `ClipPaste` / `ClipCopy` from `read_only`.
4. App test: Ctrl+X inside → no job; paste outside → Move; paste inside → Repack; archive → archive refused.
