# 216. More clouds — plan

Spec: [../specs/216-more-clouds.md](../specs/216-more-clouds.md). A PR per step, in order.

## 216a (branch feat/clouds-oauth, 0.132.0)
1. Test in `core::cloud`: `parse_remotes` maps `yandex`, `pcloud`, `gphotos`, `hidrive` types to their plugins; `fresh_name` for them.
2. `PLUGINS` rows: Yandex Disk / yandex, pCloud / pcloud, Google Photos / gphotos, HiDrive / hidrive.
3. `help-clouds` (en, ru) lists the new names and the Google Photos limits; README / README_ru Clouds section; tc-reference row 215; ROADMAP 216a; TESTING.md section; CHANGELOG; version.

## 216b (branch feat/clouds-fields, 0.133.0)
1. core: `Plugin.fields: &[Field]` (`key`, `label` fl key id, `secret`, `default`); parse of the `--non-interactive` JSON answer (`State`, `Option{Name, Help, Examples, IsPassword}`, `Error`) — tests on captured rclone 1.60 output.
2. core: `create(p, name, values) -> Step` / `answer(name, state, result) -> Step` (Step = Done | Ask(question) | Failed), secrets through `rclone obscure -`; remote removed on cancel / failure.
3. app: `Dialog::CloudAccount { plugin, values, question }`, opened by «Add account» for a plugin with fields or by a question; OK runs the step on a blocking task.
4. Plugins: Mail.ru, S3, B2, Nextcloud / WebDAV, Koofr, Seafile, Zoho, Jottacloud. Docs as in 216a.

## 216c (branch feat/clouds-newer, 0.134.0)
1. core: `parse_backends(rclone help backends)`; read with the remotes.
2. drawer: a plugin missing from it shows «Needs a newer rclone» + rclone.org/install.
3. Plugins: MEGA, Proton Drive, iCloud Drive (2FA as a 216b question). Docs.
