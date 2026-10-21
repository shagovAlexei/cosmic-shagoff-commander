# 171. Checksum files — plan

Spec: `.claude/docs/specs/171-checksums.md`.

1. core `checksum`: `hash`, `create`, `parse`, `verify`, `is_sum_file` (md-5, sha1, sha2).
   Tests: known hashes, create + verify with a changed and a missing file, separate files, GNU
   lines with `\r` / upper case / two spaces.
2. app: `Dialog::Checksum`, `Action::ChecksumCreate` / `ChecksumVerify` (Files menu, F1),
   `Message::Summed` / `Verified`, `why-no-sums`.
3. tc-reference, ROADMAP, TESTING, version 0.85.0.
