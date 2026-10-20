# 165. Command line: Ctrl+X, names not in UTF-8 — plan

Spec: `.claude/docs/specs/165-cmdline-more.md`.

1. core `cmdline::word(path, siblings)` + `glob_fits`, tests (glob, ambiguity by bytes / chars).
2. app: `cmd_action` — `ClipCut` with a line → `cmdline::next`; CmdName / CmdPath / CmdCwd through
   `word`, error `cmd-name-ambiguous`; `not_for_text` passes Ctrl+X, `FieldKey(ClipCut)` only for
   the command line. Tests.
3. help row Ctrl+X, tc-reference, ROADMAP, TESTING, version 0.79.0.
