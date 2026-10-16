# 95. Packing with a password — plan

Spec: `../specs/95-archive-encrypt.md`.

1. Core: `Packer::new(.., password)`, `pack_with`, `Format::can_encrypt` (test: contents only with the password, names without).
2. App: `dialogs::Pack.{password, again}`, `Pack::password()`, `Message::PackPassword`, `Job::Pack.password` (test).
