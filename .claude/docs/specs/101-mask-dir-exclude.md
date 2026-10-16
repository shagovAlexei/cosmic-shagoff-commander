# 101. Masks: excluding dirs

ROADMAP «Синхронизация, дальше: исключения в маске для каталогов».

- TC syntax: after `|` a pattern ending in `\` (or `/`) names dirs — `*.* | .git\ target\`.
  `Mask::skips_dir(name)`; file names never match those patterns, and dir patterns before `|` are
  dropped (the mask lists files).
- Ctrl+Shift+S: an excluded dir (on either side) gives no row and is not compared inside.
- Alt+F7: an excluded dir is not gone into (nor listed).
