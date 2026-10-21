# 174. Desktop thumbnailers — plan

Spec: `.claude/docs/specs/174-thumbnailers.md`.

1. core `terminals::exec_words` (quoting only; `exec_argv` = it + the field-code filter).
2. core `thumbnailer.rs`: `Registry::load(dirs)`, `mime_of`, `for_name`, `argv(exec, in, out, size)`; tests.
3. app `thumbs.rs`: `registry()` once, `wanted(name, size)`, `make` → `external` (O_EXCL temp file
   in /tmp, timeout, cleanup); `want_thumbs` uses `thumbs::wanted`.
4. Headless check (PDF / mp4 / SVG), docs, version 0.88.0.
