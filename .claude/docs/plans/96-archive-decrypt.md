# 96. Opening password-protected archives — plan

Spec: `../specs/96-archive-decrypt.md`.

1. Core: password store, `PASSWORD_ERROR`, zip / 7z readers and the 7z index take the password; repack keeps it (test: none / wrong → error, right → out; changed → still encrypted).
2. App: `Dialog::Password`, `Message::ArchivePassword`, job error → dialog, OK → `set_password` + Retry (test).
