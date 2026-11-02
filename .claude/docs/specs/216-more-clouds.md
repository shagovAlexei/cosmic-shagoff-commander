# 216. More clouds

Continues 215 (OneDrive, Google Drive, Dropbox, Box through `rclone mount`). Same frame: a plugin is
a row of `cloud::PLUGINS` (rclone storage type), an account is an rclone remote, mounted in
`$XDG_RUNTIME_DIR/shagoff-clouds/<plugin>:<remote>`. Three steps, a PR each.

The rclone of Pop!_OS 24.04 / Ubuntu (`apt`) is 1.60.1 (Debian build): it has `yandex`, `pcloud`,
`gphotos`, `hidrive`, `mailru`, `s3`, `b2`, `webdav`, `koofr`, `seafile`, `zoho`, `jottacloud`;
it has no `mega`, `protondrive`, `iclouddrive` (newer or dropped by Debian).

## 216a. Browser sign-in

Yandex Disk (`yandex`), pCloud (`pcloud`), Google Photos (`gphotos`), HiDrive (`hidrive`): OAuth in
the browser exactly like 215, so new `PLUGINS` rows only.

Google Photos: the API gives no original quality (images recompressed, no location data), shows
only `media/`, `album/`, `shared-album/`, `upload/`; files can only be added (to `upload/` or an
album), never changed or deleted. Said in the README and the help line.

## 216b. Fields and questions

Plugins whose sign-in is not a browser: Mail.ru Cloud (`mailru`: e-mail, app password), S3
(`s3`: endpoint, empty = AWS, region, access key, secret; provider `AWS` or `Other`), Backblaze B2
(`b2`: key id, key), Nextcloud / WebDAV (`webdav`, vendor `nextcloud`: address, user, password),
Koofr (`koofr`: e-mail, app password), Seafile (`seafile`: address, user, password), Zoho WorkDrive
(`zoho`, browser + region question), Jottacloud (`jottacloud`: a login token question).

- `Plugin.fields`: the rclone options asked before creating (key, label, secret?). Empty = browser.
- «Add account» with fields opens `Dialog::CloudAccount` (one input per field, secrets hidden).
- Secrets are obscured with `rclone obscure -` (stdin) before going on the command line.
- `rclone config create NAME TYPE k v … --non-interactive` answers JSON; a non-empty `State` is a
  question (`Option.Name`, `Option.Help`, examples): the dialog asks it, `rclone config update NAME
  --continue --state S --result R --non-interactive` answers, until `State` is empty. Cancel or
  failure removes the remote (as 215).

## 216c. Newer rclone

MEGA (`mega`: user, password), Proton Drive (`protondrive`: user, password, 2FA code optional),
iCloud Drive (`iclouddrive`: Apple ID, password, then the 2FA code as a 216b question).

- `cloud::backends()`: the types in `rclone help backends` (read with the remotes). A plugin whose
  type is missing shows «Needs a newer rclone: rclone.org/install» in place of «Add account».
