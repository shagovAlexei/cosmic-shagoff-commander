# 16. Монтирование: флешки, разделы, сеть

Статус: решения приняты по TC и по тому, что есть в системе (пользователь: «действуй»).
Основа: [архитектурный спек](00-architecture-design.md), [эталон TC](../tc-reference.md).

## Решения

| Вопрос | Решение |
|---|---|
| Механизм | Утилита `gio` (glib, есть в любой GNOME/COSMIC-системе), без библиотек glib в сборке. Флешки монтирует udisks через `gio mount -d`, сеть — gvfs через `gio mount URL` |
| Как видно сеть | gvfs отдаёт каждую сетевую точку каталогом в `$XDG_RUNTIME_DIR/gvfs/` (FUSE). Все операции (F3–F8, архивы, поиск) работают по нему как по обычному пути |
| Кнопки дисков | К дискам из `/proc/self/mounts` добавляются каталоги из `$XDG_RUNTIME_DIR/gvfs/`; подпись — хост (`sftp:host=nas,user=bob` → `nas`), для smb — `сервер/ресурс` |
| Alt+F1 / Alt+F2 | После открытия список дополняется разделами, которые можно смонтировать, но они не смонтированы (`имя  /dev/sdb1  (не подключён)`). Enter на таком — смонтировать и перейти |
| Подключиться | Ctrl+F (в TC — FTP-соединение): диалог «Адрес» (`sftp://user@host/путь`, `smb://`, `ftp://`, `dav://`) и «Пароль» (скрыт). После подключения панель переходит в точку монтирования |
| Пароль | `gio mount` спрашивает в stdout (`User [..]: `, `Domain [..]: `, `Password: `). Отвечаем сами: пользователь и домен — по умолчанию (из адреса), пароль — из поля, один раз. Второй запрос пароля — «Неверный пароль», запрос без пароля в поле — «Нужен пароль», любой другой вопрос (неизвестный ключ хоста и т. п.) — показать его текст и прервать |
| Отключить | Ctrl+Shift+F (в TC — разорвать FTP-соединение): диск активной панели, если он в `/media`, `/run/media` или gvfs (несъёмный раздел вроде `/home` udisks отключает только с паролем администратора — не трогаем); не во время операции; — `gio mount -e` (извлечь), если нельзя — `gio mount -u`. Панели, стоявшие внутри, уходят в домашний каталог |
| Ожидание | `gio` работает в фоне; в строке состояния «Подключение…» / «Отключение…»; результат или ошибка — туда же. Отмены нет (ssh сам прерывает по таймауту) |

## Ядро: `mount.rs`

```rust
pub struct Volume { pub name: String, pub device: String, pub mount: Option<PathBuf> }
/// Volumes with a unix device that can be mounted, from `gio mount -li`.
pub fn volumes(gio_list: &str) -> Vec<Volume>;
/// Drive buttons for gvfs mounts: entries of `root` (= $XDG_RUNTIME_DIR/gvfs).
pub fn gvfs_drives(root: &Path) -> Vec<Drive>;
pub fn gvfs_label(name: &str) -> String;
/// `local path:` from `gio info`.
pub fn local_path(gio_info: &str) -> Option<PathBuf>;
pub enum Error { NeedPassword, WrongPassword, Question(String), Failed(String) }
/// The answer to a `gio mount` prompt; `asked` = the password was already sent.
pub fn answer(prompt: &str, password: &str, asked: &mut bool) -> Result<String, Error>;
pub fn mount_device(device: &str) -> Result<PathBuf, Error>;
pub fn connect(url: &str, password: &str) -> Result<PathBuf, Error>;
pub fn unmount(path: &Path) -> Result<(), Error>;
```

## Тесты

Core: разбор `gio mount -li` (смонтированный, несмонтированный, без устройства, `can_mount=0`, `%20` в пути), подписи gvfs, `local path`, ответы на запросы, диалог с подставной программой вместо `gio` (пароль, неверный пароль, вопрос, ошибка).

App: Alt+F1 дополняется томами только своего открытого списка; Enter на томе запускает монтирование (не переход); Ctrl+F открывает диалог; Ctrl+Shift+F на `/` и `~` ничего не делает; после отключения панели внутри уходят домой; ошибки — в строке состояния.

Вручную (TESTING.md): флешка — вставить, Alt+F1, Enter, Ctrl+Shift+F; `sftp://localhost` с паролем и неверным паролем.

## Вне фазы

Сетевое окружение (обзор smb/avahi), сохранённые соединения и пароли (keyring), отмена подключения, MTP-телефоны как отдельная тема (работают, если gvfs их видит).
