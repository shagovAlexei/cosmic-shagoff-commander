# 90. Drag and drop in archives

ROADMAP «Drag & drop, дальше: перетаскивание мышью в архиве».

Rows inside an archive were not drag sources: other programs would have got paths that are not on
disk (`/x/a.zip/docs/f`). F5 / F6 from and into archives work already.

- Rows inside an archive are drag sources again, offered only as our own type
  `clipboard::INNER` (`application/x-shagoff-paths`, the uri-list format). Other programs do not
  take it, so the drop is refused there; our drop zones accept it first. A drop on us is the F5 /
  F6 dialog as for any drop: extraction (`OpKind::Extract`), or a move out.
- Into an archive: a drop on a pane showing an archive, or on a dir row inside it, is the F5
  dialog aimed there (repack). New: an archive file's row in a list is a drop zone too — packed
  into it, as TC.
- Not checked in headless (the drop never arrives there); by hand: TESTING § 90.
