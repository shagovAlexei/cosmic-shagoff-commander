# 08. История (Alt+←/→/↓), hotlist (Ctrl+D), обмен панелей (Ctrl+U)

Статус: одобрен.
Основа: [архитектурный спек](00-architecture-design.md), [07](07-quick-search.md), [эталон TC](../tc-reference.md).

## Решения

| Вопрос | Решение |
|---|---|
| История | Своя у каждой вкладки, до 50 записей, только в памяти. Ctrl+T копирует её в новую вкладку |
| Когда пишется | Успешный `Listed` для другого каталога. Перечитывание того же каталога историю не трогает |
| Alt+← / Alt+→ | Шаг по истории. Новый переход после шага назад обрезает «вперёд» |
| Alt+↓ | Список истории вкладки, свежие сверху, без повторов; выбор — обычный переход |
| Hotlist | Поле `hotlist` в `Config`: `Vec<HotEntry { name, path }>`; правка файла применяется на лету |
| Ctrl+D | Список: первый пункт «Добавить текущий каталог» (подпись — имя каталога, повтор пути не добавляется), дальше избранное. Enter — перейти, Delete/F8 — удалить пункт (список открыт, конфиг пишется сразу) |
| Ctrl+U | Меняет панели целиком (вкладки, история, фильтры, отметки). Фокус остаётся на той же стороне. Поле поиска закрывается |

## Ядро — `history.rs`

```rust
pub struct History { items: Vec<PathBuf>, pos: usize }   // Default, Clone, Debug
pub const LIMIT: usize = 50;
/// Now at `path`: no-op if it is the current entry; else drop "forward", push, trim to LIMIT.
pub fn visit(&mut self, path: &Path);
/// Step back / forward; the path to open, or None at the end.
pub fn back(&mut self) -> Option<PathBuf>;
pub fn forward(&mut self) -> Option<PathBuf>;
/// Unique paths, most recent first (current first).
pub fn recent(&self) -> Vec<PathBuf>;
```
`back`/`forward` сдвигают `pos` сразу; последующий `Listed` того же пути вызывает `visit`, который видит текущую запись и ничего не делает. Если каталог исчез, переход не удаётся, позиция остаётся сдвинутой (следующий `visit` обрежет «вперёд» от неё).

## Приложение

### Общий список
```rust
pub enum ListKind { Drives, History, Hotlist }
pub struct ListItem { pub label: String, pub path: PathBuf }
Dialog::List { kind: ListKind, side: usize, cursor: usize, items: Vec<ListItem> }
```
Заменяет `Dialog::Drives`: тот же снимок списка при открытии, ↑/↓/Enter/Escape, клик по пункту (`Message::ListPick(usize)`). Заголовок по виду: «Диск», «История», «Избранное».
- Drives: пункты — диски, курсор на диске текущего каталога.
- History: `History::recent()`, курсор на 0 (текущий), подпись — путь.
- Hotlist: пункт 0 — «Добавить текущий каталог», дальше `config.hotlist` (подпись — `name`, справа путь), курсор на 0.

Enter / клик:
- Hotlist, пункт 0: добавить `HotEntry { name: dir_title(cwd), path: cwd }`, если пути ещё нет; записать конфиг; закрыть список.
- Остальное: закрыть список и перейти в `path` в панели `side` (`go_drive` переименовывается в `go_to`).

`Action::Delete` / `Action::DeletePermanent` в списке Hotlist на пункте ≥ 1: удалить запись из `config.hotlist`, записать конфиг, пересобрать пункты списка, курсор ограничить.

### История во вкладке
`Tab.history: History`. В `Listed` (Ok): если путь отличается от прежнего cwd вкладки — `history.visit(path)`; первый `Listed` вкладки тоже записывается. Alt+←/→: `back()/forward()` → `load`.

### Ctrl+U
`self.panes.swap(0, 1)`, `self.search = None`, `restore_scroll` обеих панелей. `Message::Listed` ищет вкладку по `tab` id в обеих панелях (сторона из сообщения — только подсказка), иначе чтение, начатое до обмена, теряется и вкладка застревает в `pending`.

### Клавиши
keymap, блок Alt (`mods == ALT`): `ArrowLeft` → `HistoryBack`, `ArrowRight` → `HistoryForward`, `ArrowDown` → `HistoryList`. Физические: Ctrl+D (`KeyD`) → `Hotlist`, Ctrl+U (`KeyU`) → `SwapPanes`.

## Тесты

Core (`history`): первый visit; back/forward и края; повтор текущего не пишется; новый visit после back обрезает «вперёд»; предел 50 (самые старые уходят, позиция корректна); `recent` без повторов, свежие первыми.

App: два перехода + Alt+← открывает первый каталог, Alt+→ — второй; перечитывание не пишет историю; Alt+↓ открывает список истории; Ctrl+D «добавить» пишет текущий каталог один раз; Enter на пункте hotlist переходит; Delete удаляет пункт; Ctrl+U меняет вкладки местами, а `Listed`, начатый до обмена, применяется; диски через `Dialog::List` работают как раньше (существующие тесты).

keymap: Alt+←/→/↓, Ctrl+D (лат./кир.), Ctrl+U; Alt+Shift+← → None.

## Вне фичи

Сохранение истории между запусками, переименование и сортировка hotlist в диалоге, подменю hotlist.
