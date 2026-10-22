# 175. Thumbnailer sandbox — plan

Spec: `.claude/docs/specs/175-thumb-sandbox.md`.

1. core `thumbnailer::{bwrap_base, sandboxed, bwrap_works}` + test (skipped without bwrap): `cp %i %o`
   works, the input's real dir isn't visible.
2. app `thumbs::external`: bwrap path when `bwrap_works()` (once), else the 174 path.
3. Headless check, docs, version 0.89.0.
