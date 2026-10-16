# 92. From archive to archive

ROADMAP «Архив, дальше: копирование из архива в архив».

F5 / F6 / Ctrl+V / drag and drop from an archive into another archive (or into another dir of the
same one) said «read-only».

- `repack::cross`: extract the names into a staging dir next to the target archive, add them there
  (`modify(Add)`, conflicts asked as for F5 into an archive), with F6 then delete from the source
  archive what was added completely; staging removed in every case.
- Inside one archive: the same dir → «same file», a dir into itself → «into itself», as on disk.
- `Job::Cross`, shown as a repack.
