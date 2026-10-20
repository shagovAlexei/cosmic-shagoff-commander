# 165. Command line: Ctrl+X, names not in UTF-8

ROADMAP «Командная строка, дальше».

- Ctrl+X in the command line on a line from the history (Ctrl+E brought it): the next command
  (TC, the pair of Ctrl+E), `cmdline::next`; at the newest the line stays. A typed or edited line
  stays too: libcosmic's field does its own cut (selection to the clipboard) first, and that is
  all. Text fields now pass Ctrl+X on (`not_for_text`); only the command line acts on it — quick
  search, the viewer, dialogs and settings ignore it. The panel's Ctrl+X, the menu's «Cut», a
  toolbar button or a hotkey cut the files as before, whatever the line holds.
- Ctrl+Enter / Ctrl+Shift+Enter / Ctrl+P with a name not in UTF-8 (the line is text): each such
  byte becomes a glob `?` outside the quotes (`'caf'?'.txt'`; dash has no `$'\xNN'`), works in
  dash, bash and zsh. When the pattern fits another name in that dir — by bytes (dash) or by
  characters (bash, zsh) — nothing is inserted and the status line says why
  (`cmdline::word`; the names are read with `read_dir`, a dir that cannot be read — inside an
  archive — inserts nothing). In search results / branch Ctrl+Enter inserts the path below the
  root, as shown.
- Not done: `cd` typed with such a pattern (our own `cd` takes the word literally).
