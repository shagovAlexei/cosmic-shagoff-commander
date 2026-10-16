# 101. Masks: excluding dirs

ROADMAP «Синхронизация, дальше: исключения в маске для каталогов».

- TC syntax: after `|` a pattern ending in `\` (or `/`) names dirs — `*.* | .git\ target\`.
  `Mask::skips_dir(name)`, by the name only (as TC); file names never match those patterns. Before
  `|` such a pattern stays as it was (matches no file).
- Ctrl+Shift+S: an excluded dir (on either side) gives no row and is not compared inside.
- Alt+F7: an excluded dir is not gone into, nor listed; inside archives members under it are left out too.
