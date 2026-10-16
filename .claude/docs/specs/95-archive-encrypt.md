# 95. Packing with a password

ROADMAP «Архив, дальше: шифрование». Part 1 of 2: packing. Part 2: opening / extracting
password-protected archives (asks the password).

- Alt+F5 for zip / 7z: «Пароль» and «Пароль ещё раз» (as TC: a typo would lock the files away);
  they differ → a note, OK does nothing. Empty = not encrypted. Tar formats: no fields.
- `archive::pack_with(.., password, ..)`: zip — AES-256 per file (`zip` feature `aes-crypto`);
  7z — AES-256 + LZMA2, header not encrypted. Names stay readable in both, as 7-Zip by default.
