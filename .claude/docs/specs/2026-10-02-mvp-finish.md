# Фаза 6: F3/F4, диски, watcher, конфиг. MVP готов

Дата: 2026-10-02. Статус: одобрен.
Основа: [архитектурный спек](2026-10-02-architecture-design.md), [фаза 5](2026-10-02-file-ops.md), [эталон TC](../tc-reference.md).

## Решения

| Вопрос | Решение |
|---|---|
| Формат конфига | cosmic-config (стандарт COSMIC): файл на поле, версия схемы, правка файла применяется на лету |
| Настройки и состояние | Раздельно: `Config` в `~/.config/cosmic/<APP_ID>/v1/` (правит человек), `State` в state-каталоге COSMIC `~/.local/state/cosmic/<APP_ID>/v1/` (пишет программа) |
| F3 | `viewer` из конфига, по умолчанию пусто, то есть `xdg-open` (программа по типу файла) |
| F4 | `editor` из конфига, по умолчанию `cosmic-edit` |
| F3/F4 на каталоге или `..` | Ничего не делать |
| Диски | `/`, `~` и реальные устройства из `/proc/self/mounts` |
| Ctrl+H | Одна настройка на окно, хранится в `Config`, перечитывает все вкладки обеих панелей |
| Ctrl+W на последней вкладке | `last_tab_close`: `Nothing` (по умолчанию) или `Home`, то есть перейти в `home_dir`, а если он не задан — в `~` |
| Watcher | Только cwd активной вкладки каждой панели, без рекурсии. Пауза, пока идёт операция |

## Конфиг — `crates/app/src/config.rs`

```rust
#[derive(Clone, Debug, PartialEq, CosmicConfigEntry, Serialize, Deserialize)] // Default вручную
#[version = 1]
pub struct Config {
    pub show_hidden: bool,
    pub viewer: Vec<String>,        // пусто → ["xdg-open"]
    pub editor: Vec<String>,        // Default: ["cosmic-edit"]
    pub last_tab_close: LastTab,    // Nothing | Home
    pub home_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, CosmicConfigEntry, Serialize, Deserialize)]
#[version = 1]
pub struct State {
    pub panes: [PaneState; 2],
    pub active: usize,
}
// core::session
pub struct PaneState { pub tabs: Vec<PathBuf>, pub active: usize }
```

- `Default` для `Config` реализован вручную из-за `editor`. `PaneState` живёт в core, поэтому core получает зависимость `serde`. Чтение: `get_entry(...).unwrap_or_else(|(_, c)| c)`, поэтому битый или старый конфиг не мешает запуску.
- Изменения `Config` приходят через `core.watch_config::<Config>(APP_ID)`. Если поменялся `show_hidden`, перечитываются все вкладки.
- `State` пишется при каждой смене каталога, вкладки или активной панели, но только если снимок отличается от предыдущего.
- Команды хранятся списком аргументов без shell. Путь к файлу добавляется последним аргументом.

## Ядро

### `session.rs`

```rust
pub struct PaneState { pub tabs: Vec<PathBuf>, pub active: usize }  // serde
/// Ближайший существующий каталог: сам путь, иначе родитель, …, иначе `fallback`.
pub fn existing_dir(path: &Path, fallback: &Path) -> PathBuf;
/// Пути для восстановления: каждый через `existing_dir`, пустой список → [fallback], active ограничен длиной.
pub fn restore(state: &PaneState, fallback: &Path) -> (Vec<PathBuf>, usize);
```

`snapshot` собирает `PaneState` в `app.rs` из `Tabs<Tab>`, это две строки, отдельной функции в core нет. Аргумент командной строки заменяет путь активной вкладки левой панели.

### `drives.rs`

```rust
pub struct Drive { pub label: String, pub path: PathBuf }
pub fn parse(mounts: &str, home: &Path) -> Vec<Drive>;
/// Индекс диска с самым длинным префиксом пути (по компонентам).
pub fn containing(drives: &[Drive], path: &Path) -> Option<usize>;
```

Правила `parse`:
1. Сначала `/` (подпись `/`) и `home` (подпись `~`).
2. Затем строки, где устройство начинается с `/dev/`, кроме `/dev/loop*`, ФС `squashfs` и точек `/boot` и `/boot/*`.
3. Точка `/` повторно не добавляется. Точка, совпадающая с `home`, тоже.
4. Одно устройство в нескольких точках — одна кнопка: точка под `/media/` или `/run/media/`, иначе первая.
5. Подпись — последний компонент точки. Экранирование в mounts (`\040` — пробел) раскодируется.
6. Порядок — как в mounts.

`containing` для `~/x` выбирает `~`, а не `/home`, потому что префикс длиннее.

### `launch.rs`

```rust
/// argv для F3/F4: команда из конфига + путь; пустая команда → `default`.
pub fn command(cmd: &[String], default: &[&str], file: &Path) -> Vec<OsString>;
```

## Приложение

### F3 / F4
- `Action::View` (F3) и `Action::Edit` (F4) берут файл под курсором, `Kind::File`. На каталоге и `..` ничего не происходит.
- Запуск через `Command::spawn`, ожидание в отдельном потоке, чтобы не оставлять зомби (как `open_detached`). Ошибка (нет программы) показывается в строке состояния текстом из `fl!`.
- Кнопки F3 и F4 в нижнем ряду становятся активными.

### Кнопки дисков
- Над каждой панелью: ряд кнопок дисков. Подсвечена кнопка из `drives::containing(cwd)`. Справа текст `fl!("disk-free", free, total)` — «12,3 ГБ свободно из 450 ГБ», через `rustix::fs::statvfs` (rustix уже в lock).
- Клик по кнопке переводит активную вкладку этой панели в корень диска и делает панель активной.
- Список дисков и место на диске перечитываются после каждого `Listed` в этой панели. Чтение `/proc/self/mounts` и statvfs — синхронно, это микросекунды.
- Alt+F1 / Alt+F2 открывают `Dialog::Drives { side, cursor }`: список дисков, ↑/↓, Enter — перейти, Escape — закрыть. В keymap правило «Alt → None» пропускает Alt+F1 и Alt+F2. Если COSMIC перехватывает эти клавиши — записать в tc-reference и TESTING.md, клик по кнопкам остаётся.

### Watcher — `crates/app/src/watcher.rs`
- `notify::RecommendedWatcher` в `Subscription`, по одной на панель, ключ — путь cwd активной вкладки. Смена пути пересоздаёт подписку.
- Любое событие ставит таймер 200 мс, повторные события его не продлевают дольше 1 с. По таймеру — `Message::Changed(side)`, который перечитывает активную вкладку этой панели. Курсор и отметки сохраняет `Panel::set_listing` (тот же каталог).
- Пока `job` выполняется, подписки нет. `finish_job` перечитывает обе панели, как сейчас.
- Неактивная вкладка перечитывается при `SelectTab` и Ctrl+Tab.
- Ошибка notify (нет inotify-слотов, каталог удалён) пишется в лог, панель работает без автообновления.

### Поколения вместо `pending: PathBuf` (техдолг)
`Tab.pending: Option<u64>` — номер запроса. `Listed` применяется, только если его номер равен `pending`. Два чтения одного пути больше не теряют новое.

### Ctrl+H, Ctrl+W
- `Action::ToggleHidden` (Ctrl+H, физическая `KeyH`): переключает `show_hidden`, пишет в `Config`, перечитывает все вкладки.
- `Action::CloseTab` на последней вкладке: при `Home` вкладка уходит в `home_dir` или `~`, при `Nothing` ничего не происходит.

## Тесты

Unit (core):
- `drives::parse` на тексте mounts с этой машины: 22 snap `loop`, `/boot/efi`, `sda1` в `/mnt/...` и `/media/...` → `[/, ~, home, SAVE_FLASH, sys]`; `\040` в имени; `/` и home без повторов.
- `drives::containing`: `~/x` → `~`; `/media/shag/sys/a` → `sys`; `/etc` → `/`; `/homework` не попадает в `/home`.
- `session::existing_dir` и `restore`: удалённый путь → родитель; всё удалено → fallback; пустой список; `active` за пределами.
- `launch::command`: пустая команда → default + файл; команда с аргументами → аргументы + файл.
- keymap: F3, F4, Ctrl+H (в т.ч. русская раскладка), Alt+F1, Alt+F2; прочие Alt по-прежнему `None`.

Вручную (TESTING.md «Фаза 6»): запоминание вкладок, правка конфига на лету, F3/F4, кнопки дисков и место, флешка, Alt+F1/F2, автообновление при `touch`/`rm` из терминала, отсутствие дёрганья при копировании большого файла, Ctrl+H, Ctrl+W на последней вкладке с обеими опциями.

## Вне фазы

Shift+F4 (создать файл), Alt+F4 через приложение, окно настроек, рекурсивный watcher, слежение за неактивными вкладками.
