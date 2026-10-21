# 171. Checksum files (Files → Create / Verify checksums)

TC «Create CRC checksum files» (`cm_CRCcreate`) and «Verify checksums» (`cm_CRCcheck`), missing until now.

- Create: a dialog over the targets: algorithm MD5 / SHA1 / SHA256 / SHA512 (kept in
  `Config.checksum_algo`), «a separate file for each file». One file `<dir name>.<ext>` with every
  file of the targets (dirs walked, links and special files skipped, names relative to the panel's
  dir), or `<name>.<ext>` per file. GNU format `hash *name`, readable by `sha256sum -c`. A taken
  name is an error, never overwritten (part file + rename without replacing). Background; leaving
  the dir stops it. Then both panels reread, the cursor on the new file.
- Verify: the sum files among the targets (`.md5 .sha1 .sha256 .sha512 .sha`, `*SUMS`); the
  algorithm by the hash's length; names relative to the sum file. Result in the status line: all
  intact, or «N of M failed» with a line per bad / missing file.
- Not inside an archive (Create is refused there; Verify skips such targets).
- No default keys (TC has none either); Files menu, F1, own hotkeys.
