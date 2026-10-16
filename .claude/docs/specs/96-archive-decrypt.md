# 96. Opening password-protected archives

Part 2 of the archive encryption (95: packing). Extracting from an encrypted zip / 7z failed with
«Password required to decrypt file».

- Core: `archive::set_password` / `password` — kept per archive (real path) until the program exits,
  never written. zip reads with `by_index_decrypt`, 7z with the password; missing or wrong →
  `PASSWORD_ERROR` (`needs_password`). A wrong 7z password only shows as a decoder error: with a
  password given, any read error counts as a wrong one.
- Changing an encrypted archive (F5 / F6 in, F7, F8, Shift+F6, Ctrl+M, F4) packs it again with the
  same password.
- App: that error from a job opens «Пароль архива» (masked field; «Неверный пароль» when one was
  given); OK keeps it and retries, Cancel / Esc cancels the job.
- A 7z with encrypted names (7-Zip's option): entering it asks the same way, then opens it again.
- Ctrl+Q quick view uses a password already given; before that it shows nothing (not asked).
