# 210 — Update check at start

At start (once a day, in the background) the program asks GitHub for the latest release (`core::update::check`, `curl`, the same as About → «Check for updates»). A newer one opens a dialog: **Update**, **Later**, **Skip this version** (kept in `State.update_skip`), and a «Support the project» button with the ask (one McDonald's lunch or a couple of cups of coffee).

- Off with Settings → «Check for updates at start» (`Config.check_updates`, default on). A failed check (no network) is silent and does not spend the day (`State.update_checked`).
- **Update** downloads the release's `.deb`, checks its SHA-256 against the release's `SHA256SUMS` (a missing or mismatching sum refuses), and installs with `pkexec apt-get install -y`. Only on an x86-64 system with `apt-get` and `pkexec`; elsewhere (and on any failure) the dialog offers the release page.
- The tag is only taken when it reads `vN.N.N` (`update::newer`), so it is safe in a URL and a file name.
- Release notes are not shown (the release page has them): a JSON string with escapes needs a parser the program does not carry.
