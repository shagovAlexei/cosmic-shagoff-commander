# 91. Clipboard in archives

ROADMAP «Архив, дальше: Ctrl+V / Ctrl+X … в архиве».

Ctrl+X and Ctrl+V inside an archive were refused as read-only; Ctrl+C there extracted straight into
the other panel. F5 / F6 out of and into archives work already (`start_transfer`).

- Ctrl+C / Ctrl+X inside an archive put the paths on the clipboard only as our own type
  (`clipboard::INNER`, now in the gnome layout so it carries copy / cut): no other program may take
  paths that are not on disk.
- Ctrl+V in our panes goes the F5 / F6 way: out of an archive → extract (cut → move out, deleted from
  the archive); into an archive → pack (cut → sources deleted after packing); archive → archive →
  «read-only» at once, as for drag and drop (90).
- Paste into Files from an archive: nothing (same as a drag there).
