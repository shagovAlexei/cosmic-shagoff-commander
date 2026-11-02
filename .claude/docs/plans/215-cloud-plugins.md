# 215. Плагины облаков — план

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** OneDrive, Google Drive, Dropbox, Box как диски (Alt+F1, кнопки дисков) через `rclone mount`, с переключателем каждого плагина и входом / удалением учётной записи в Ctrl+, → «Облака».

**Architecture:** `core::cloud` — таблица плагинов + чистые разборы (`rclone listremotes --long`, `/proc/self/mounts`) + тонкие обёртки над `rclone` / `fusermount`. `core::mount` подмешивает облачные тома (`device` = `rclone:<имя>`) в `list()` и отправляет `rclone:` в `cloud::mount`, так что Alt+F1, кнопки дисков и Ctrl+Shift+F работают без новых путей в app. App фильтрует по `Config.plugins_off` и рисует раздел настроек.

**Tech Stack:** Rust 2024, libcosmic, `rclone` 1.60 CLI, `fusermount`, cosmic-config, Fluent.

**Spec:** [.claude/docs/specs/215-cloud-plugins.md](../specs/215-cloud-plugins.md)

## Global Constraints

- Без новых зависимостей crate; `rclone` — внешняя программа, ставит пользователь (`sudo apt install rclone`).
- Корень точек: `$XDG_RUNTIME_DIR/shagoff-clouds`, точка `<id>:<remote>`, каталоги 0700.
- `rclone mount <remote>: <точка> --daemon --vfs-cache-mode writes`.
- Имя новой учётной записи: id плагина, занятое → `<id>-2`, `<id>-3`…
- `Config.plugins_off: Vec<String>`, `Default` пусто.
- Отрисовка не трогает диск и не запускает rclone: учётные записи читаются задачей при открытии настроек и после добавления / удаления.
- Строки — `fl!` в `en` и `ru`. Ветка `feat/cloud-plugins`, версия minor, CHANGELOG, README.md + README_ru.md.

## Review Focus

1. Remote пользователя с пробелом или точкой в имени (`my drive`, `work.od`) — точка монтирования, подпись, `rclone mount "my drive:"` (аргумент одним элементом, без `sh`).
2. `rclone` не установлен — `mount::list()` не падает и не тормозит (одна проверка `PATH`), раздел показывает команду установки.
3. Esc во время входа — процесс rclone и открытый им веб-сервер убиты (группа процессов), повторное «Добавить» не получает «address already in use».
4. Точка монтирования осталась каталогом после падения rclone (`Transport endpoint is not connected`) — повторный Enter: `fusermount -u` перед `mount`, не ошибка «каталог не пуст».
5. Выключение плагина в настройках сразу убирает его тома и кнопки (без Ctrl+R и перезапуска).

---

### Task 0: Ручная проверка (вместе с пользователем, до кода)

- [ ] Пользователь: `sudo apt install rclone`. (GOA больше не нужен: `sudo apt remove gnome-online-accounts-gtk gnome-online-accounts` и учётку Google в нём — по желанию.)
- [ ] `rclone config create gdrive drive` — браузер открылся, после входа команда завершилась 0; то же `onedrive onedrive` (если спросит тип / диск — записать в спек, риск 1, и какие параметры нужны).
- [ ] `rclone listremotes --long` — сохранить вывод как образец.
- [ ] `mkdir -p -m700 $XDG_RUNTIME_DIR/shagoff-clouds/gdrive:gdrive && rclone mount gdrive: $XDG_RUNTIME_DIR/shagoff-clouds/gdrive:gdrive --daemon --vfs-cache-mode writes`; `ls` — настоящие имена; `grep rclone /proc/self/mounts` — сохранить строку как образец; `fusermount -u …`.

### Task 1: `core::cloud` — таблица и чистые разборы

**Files:**
- Create: `crates/core/src/cloud.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod cloud;`)

**Interfaces:**
- Produces:
  - `pub struct Plugin { pub id: &'static str, pub name: &'static str, pub kind: &'static str }`; `pub const PLUGINS: &[Plugin]` — по таблице спека (`onedrive`/`OneDrive`/`onedrive`, `gdrive`/`Google Drive`/`drive`, `dropbox`/`Dropbox`/`dropbox`, `box`/`Box`/`box`)
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Remote { pub name: String, pub plugin: &'static Plugin }`
  - `pub fn parse_remotes(listremotes_long: &str) -> Vec<Remote>` — строки `name:   type`; чужие типы пропускаются
  - `pub fn fresh_name(p: &Plugin, taken: &[Remote]) -> String`
  - `pub fn label(p: &Plugin, remote: &str) -> String` — `"OneDrive"` если `remote == p.id`, иначе `"OneDrive: work"`
  - `pub fn point_name(r: &Remote) -> String` — `"onedrive:work"`; `pub fn of_point(name: &str) -> Option<(&'static Plugin, &str)>`
  - `pub fn mounted(mounts: &str, root: &Path) -> Vec<PathBuf>` — точки `fuse.rclone` прямо под `root` (имя через `drives::unescape`, `\040` = пробел)
  - `pub fn volumes(remotes: &[Remote], mounts: &str, root: &Path) -> Vec<Volume>` — `Volume { name: label(..), device: format!("rclone:{name}"), mount: Some("file://<точка>") если смонтировано }`
  - `pub fn drives(mounts: &str, root: &Path) -> Vec<Drive>` — по одному на смонтированную точку, `label` из `of_point`
  - `pub fn hidden_volume(v: &Volume, remotes: &[Remote], off: &[String]) -> bool` — `true` только для `rclone:<имя>` выключенного плагина (плагин — по `remotes`)
  - `pub fn hidden_drive(path: &Path, root: &Path, off: &[String]) -> bool` — `true` только для точки под `root` выключенного плагина (плагин — по `of_point`)
  - `pub fn root() -> Option<PathBuf>` — `$XDG_RUNTIME_DIR/shagoff-clouds`

- [ ] **Step 1: тесты** (образцы из Task 0 подставить, если формат другой)

```rust
const REMOTES: &str = "gdrive:   drive\nmy drive: drive\nnas:      sftp\nonedrive: onedrive\n";

#[test]
fn parse_remotes_keeps_plugin_kinds() {
    let r = parse_remotes(REMOTES);
    assert_eq!(r.iter().map(|r| (r.name.as_str(), r.plugin.id)).collect::<Vec<_>>(),
               [("gdrive", "gdrive"), ("my drive", "gdrive"), ("onedrive", "onedrive")]);
}

#[test]
fn fresh_name_skips_taken() {
    let r = parse_remotes(REMOTES);
    assert_eq!(fresh_name(&PLUGINS[0], &r), "onedrive-2");
    assert_eq!(fresh_name(&PLUGINS[2], &r), "dropbox");
}

#[test]
fn labels_and_points() {
    let r = parse_remotes(REMOTES);
    assert_eq!(label(r[0].plugin, "gdrive"), "Google Drive");
    assert_eq!(label(r[1].plugin, "my drive"), "Google Drive: my drive");
    assert_eq!(point_name(&r[1]), "gdrive:my drive");
    assert_eq!(of_point("gdrive:my drive").map(|(p, n)| (p.id, n)), Some(("gdrive", "my drive")));
    assert!(of_point("sftp:host=nas").is_none());
}

#[test]
fn volumes_and_drives_from_mounts() {
    let root = Path::new("/run/user/1000/shagoff-clouds");
    let mounts = "gdrive: /run/user/1000/shagoff-clouds/gdrive:gdrive fuse.rclone rw,nosuid 0 0\n\
                  /dev/sda1 /media/x ext4 rw 0 0\n";
    let v = volumes(&parse_remotes(REMOTES), mounts, root);
    assert_eq!(v[0].device, "rclone:gdrive");
    assert_eq!(v[0].mount.as_deref(), Some("file:///run/user/1000/shagoff-clouds/gdrive:gdrive"));
    assert_eq!(v[1].mount, None); // my drive
    let d = drives(mounts, root);
    assert_eq!((d.len(), d[0].label.as_str()), (1, "Google Drive"));
}

#[test]
fn hidden_only_for_switched_off() {
    let r = parse_remotes(REMOTES);
    let off = vec!["gdrive".to_string()];
    let root = Path::new("/r/c");
    let v = |d: &str| Volume { name: String::new(), device: d.into(), mount: None };
    assert!(hidden_volume(&v("rclone:gdrive"), &r, &off));
    assert!(!hidden_volume(&v("rclone:onedrive"), &r, &off));
    assert!(!hidden_volume(&v("/dev/sdb1"), &r, &off));
    assert!(hidden_drive(Path::new("/r/c/gdrive:x"), root, &off));
    assert!(!hidden_drive(Path::new("/media/x"), root, &off));
}
```

- [ ] **Step 2:** `cargo test -p shagoff-core cloud` — FAIL.
- [ ] **Step 3:** реализовать по Interfaces (`drives::unescape` сделать `pub(crate)`, если он приватный).
- [ ] **Step 4:** `cargo test -p shagoff-core cloud` — PASS.
- [ ] **Step 5:** commit `feat(core): cloud plugins table and rclone parsing`.

### Task 2: команды rclone и встраивание в `core::mount`

**Files:**
- Modify: `crates/core/src/cloud.rs`, `crates/core/src/mount.rs` (`list` ~l.402, `mount_device` ~l.407, `unmount` ~l.462, `removable` ~l.255)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub fn installed() -> bool` — `rclone` в `PATH` (обход `PATH`, без запуска)
  - `pub fn remotes() -> Vec<Remote>` — `rclone listremotes --long`, ошибка → пусто; без `installed()` — пусто без запуска
  - `pub fn add(p: &Plugin, name: &str, cancel: &AtomicBool) -> Result<(), mount::Error>` — `rclone config create <name> <kind>`, `process_group(0)`, опрос `try_wait` раз в 100 мс, отмена → `kill_process_group` + `Error::Cancelled`; ненулевой выход → `Failed(stderr)`. rclone записывает remote в конфиг **до** входа (проверено: снятый на «Waiting for code…» оставил `gdrive: drive` с пустым токеном, `mount` → «empty token found») — поэтому при отмене и ошибке `add` вызывает `forget(name)`
  - `pub fn forget(name: &str) -> Result<(), mount::Error>` — `rclone config delete <name>`
  - `pub fn mount(name: &str) -> Result<PathBuf, mount::Error>` — корень и точка 0700 (`DirBuilder::mode`), `fusermount -u -z` точки молча (Review Focus 4), затем `rclone mount … --daemon --vfs-cache-mode writes`
  - `pub fn unmount(point: &Path) -> Result<(), mount::Error>` — `fusermount -u`, затем `remove_dir`
  - `mount::list()` = gio-тома + `cloud::volumes(&cloud::remotes(), mounts, root)`
  - `mount::mount_device("rclone:<имя>")` → `cloud::mount`
  - `mount::unmount(p)` при `p` под `cloud::root()` → `cloud::unmount`
  - `mount::removable(root, gvfs)` → ещё `true` под `cloud::root()` (не сам корень) — сигнатура не меняется

- [ ] **Step 1: тест** `removable_takes_cloud_points` в `mount.rs`: путь `<cloud::root()>/gdrive:gdrive` — `true`, сам корень — `false` (переменную `XDG_RUNTIME_DIR` не трогать: строить путь от `cloud::root()`, пропустить тест, если `None`).
- [ ] **Step 2:** FAIL. **Step 3:** реализовать. **Step 4:** `cargo test -p shagoff-core` — PASS.
- [ ] **Step 5:** руками (rclone из Task 0): маленький `#[test] #[ignore] fn live_mount()` не добавлять — проверка в Task 4 через приложение.
- [ ] **Step 6:** commit `feat(core): mount clouds through rclone`.

### Task 3: приложение — фильтр, кнопки дисков

**Files:**
- Modify: `crates/app/src/config.rs` (`plugins_off`)
- Modify: `crates/app/src/app.rs` (`App.clouds: Vec<cloud::Remote>`), `crates/app/src/app/settings.rs` — `set_drives` (~l.263: `extend(cloud::drives(&m, &root))`), `apply_config` (при смене `plugins_off` → `set_drives()` + `refresh_volumes(true)`)
- Modify: `crates/app/src/app/handle.rs` — `Message::AllVolumes` (~l.2259) и `Message::Volumes` (~l.2275): отбросить `cloud::hidden_volume`; `set_drives` отбрасывает `cloud::hidden_drive`
- `App.clouds` обновляется там же, где `refresh_volumes`: тома приходят из `mount::list()`, а `remotes` — отдельной задачей `blocking(cloud::remotes, Message::Clouds)` рядом с ней
- Test: `crates/app/src/app/tests.rs`

- [ ] **Step 1: тест**

```rust
#[test]
fn switched_off_plugin_hides_its_volumes() {
    let mut app = test_app(); // как соседние тесты
    let _ = app.update(Message::Clouds(cloud::parse_remotes("gdrive: drive\nonedrive: onedrive\n")));
    app.config.plugins_off = vec!["gdrive".into()];
    let _ = app.update(Message::AllVolumes(vec![
        volume("Google Drive", "rclone:gdrive", None),
        volume("OneDrive", "rclone:onedrive", None),
    ]));
    assert_eq!(app.volumes.iter().map(|v| v.device.as_str()).collect::<Vec<_>>(), ["rclone:onedrive"]);
}
```

- [ ] **Step 2:** FAIL. **Step 3:** реализовать. **Step 4:** PASS.
- [ ] **Step 5:** commit `feat: cloud drives in the drive list, hidden when switched off`.

### Task 4: раздел «Облака» в настройках

**Files:**
- Modify: `crates/app/src/drawer.rs` — `Setting::Plugin(&'static str, bool)`; `SettingsForm.rclone: bool` (`cloud::installed()` при создании формы, рядом с `fonts`); секция после сетевых настроек: нет rclone → `text_input` только для чтения `sudo apt install rclone` с подписью; иначе для каждого `PLUGINS`: `settings::item(name, toggler)`, если включён — строки `App.clouds` этого плагина (`имя` + кнопка «Удалить» → `Message::CloudForget(name)`) и кнопка «Добавить учётную запись» → `Message::CloudAdd(id)`
- Modify: `crates/app/src/app.rs` — `Message::{Clouds(Vec<Remote>), CloudAdd(&'static str), CloudAdded(Result<(), mount::Error>), CloudForget(String), CloudForgot(Result<(), mount::Error>)}`, `App.cloud_cancel: Option<Arc<AtomicBool>>`
- Modify: `crates/app/src/app/handle.rs` — `CloudAdd`: занято (`cloud_cancel` есть или `busy()`) → ничего; иначе `say(Busy, cloud-signing-in)`, `blocking(cloud::add(..))`; `CloudAdded`: `cloud_cancel = None`, ошибка → `mount_error` (Cancelled → `connect-cancelled`), успех → `say(Note, cloud-added)`; обе ветки → `Clouds` + `refresh_volumes(true)`. `CloudForget`: если смонтирован — `cloud::unmount`, затем `cloud::forget`, затем то же обновление. Esc при `cloud_cancel` → `store(true)` (там, где Esc отменяет Ctrl+F — `act.rs`)
- Modify: `crates/app/src/app/settings.rs` — `Setting::Plugin(id, on)` → `plugins_off` без дублей
- Modify: `crates/app/i18n/{en,ru}/shagoff-commander.ftl`:
  - `set-clouds` = Clouds / Облака
  - `set-clouds-need-rclone` = Clouds need rclone: / Для облаков нужен rclone:
  - `cloud-add` = Add account / Добавить учётную запись
  - `cloud-forget` = Remove / Удалить
  - `cloud-signing-in` = Signing in in the browser… Esc to cancel / Вход в браузере… Esc — отмена
  - `cloud-added` = { $name } added: it is in the drive list (Alt+F1) / { $name } добавлен: он в списке дисков (Alt+F1)
- Test: `crates/app/src/app/tests.rs`

- [ ] **Step 1: тест**

```rust
#[test]
fn plugin_toggle_round_trips_through_config() {
    let mut app = test_app();
    let _ = app.set(Setting::Plugin("gdrive", false));
    let _ = app.set(Setting::Plugin("gdrive", false));
    assert_eq!(app.config.plugins_off, ["gdrive"]);
    let _ = app.set(Setting::Plugin("gdrive", true));
    assert!(app.config.plugins_off.is_empty());
}
```

- [ ] **Step 2:** FAIL. **Step 3:** реализовать. **Step 4:** PASS.
- [ ] **Step 5:** headless (`SHAGOFF_HEADLESS=<scratchpad>/hl headless.sh start`, Ctrl+, → «Облака», `shot clouds`): переключатели и кнопки не обрезаны на 1036×530; без rclone — строка установки.
- [ ] **Step 6:** живьём на машине пользователя (rclone и учётки из Task 0): Alt+F1 → «Google Drive (не подключён)» → Enter → настоящие имена; F5 файла туда и обратно; Ctrl+Shift+F — кнопка пропала, каталог точки удалён.
- [ ] **Step 7:** commit `feat: clouds section in settings: switch, add and remove accounts`.

### Task 5: документы и выпуск

**Files:** `.claude/docs/tc-reference.md` (строка «Облака»), `crates/app/src/help.rs` (строка в разделе дисков), `TESTING.md` (раздел «Облака (215)» в конец, рецептом: установка rclone, вход, Alt+F1 → Enter, F5 / F6 / F7 / F8, Ctrl+Shift+F, выключить плагин, Esc при входе, удалить учётку; уборка: `rclone config delete …`), `ROADMAP.md` (215, 215a–c отметить; строка про «rclone вместо gvfs»), `CLAUDE.md` (строка про `cloud.rs`), `README.md` + `README_ru.md` (облака; зависимость rclone как необязательная), `crates/app/Cargo.toml` — `rclone` в Suggests / Recommends deb и rpm, если там такие поля есть, `CHANGELOG.md`, версия minor.

- [ ] `just verify` — PASS.
- [ ] `/code-review`, исправить подтверждённое.
- [ ] commit `docs: cloud plugins (215)`; PR по `shagoff-feature`.
