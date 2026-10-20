# 165. Command line: Ctrl+X, names not in UTF-8

ROADMAP «Командная строка, дальше».

- Ctrl+X with a line typed (TC: next command, the pair of Ctrl+E): `cmdline::next`, at the newest
  the line stays; it never falls through to cutting the files. With an empty line Ctrl+X cuts the
  files as before. Text fields now pass Ctrl+X on (`not_for_text`); only the command line acts on
  it — quick search, dialogs and settings ignore it. libcosmic's field still does its own cut
  (selection to the clipboard, or deletes the character after the cursor) first.
- Ctrl+Enter / Ctrl+Shift+Enter / Ctrl+P with a name not in UTF-8 (the line is text): each such
  byte becomes a glob `?` outside the quotes (`'caf'?'.txt'`; dash has no `$'\xNN'`), works in
  dash, bash and zsh. When the pattern fits another name in that dir — by bytes (dash) or by
  characters (bash, zsh) — nothing is inserted and the status line says why
  (`cmdline::word`, the panel's own names or `read_dir` for the dirs above).
- Not done: `cd` typed with such a pattern (our own `cd` takes the word literally).
