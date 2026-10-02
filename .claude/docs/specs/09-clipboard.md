# 09. Буфер обмена Ctrl+C / Ctrl+X / Ctrl+V, совместимый с cosmic-files

Статус: одобрен.
Основа: [архитектурный спек](00-architecture-design.md), [операции](05-file-ops.md), [эталон TC](../tc-reference.md).
Формат взят из `pop-os/cosmic-files` `src/clipboard.rs` (GPL-3.0-only).

## Решения

| Вопрос | Решение |
|---|---|
| Ctrl+V | Вставляет сразу, без диалога, в текущий каталог активной панели (как TC и cosmic-files) |
| Вставка в тот же каталог | Ошибка `plan-same-file`, как у F5. Копии «(copy)» не делаем |
| Вырезанные файлы после вставки | Буфер очищается при старте переноса; повторный Ctrl+V ничего не делает |
| Ctrl+C/X/V в поле ввода | Остаются за полем (уже так: `not_for_text`) |

## Формат

| MIME | Содержимое |
|---|---|
| `x-special/gnome-copied-files` | `copy` или `cut`, затем `\n` + `file://` URI на каждый путь |
| `text/uri-list` | URI, каждый + `\r\n` |
| `text/plain`, `text/plain;charset=utf-8`, `UTF8_STRING` | пути через `\r\n` (не-UTF-8 пути пропускаются) |

Чтение принимает `x-special/gnome-copied-files` и `text/uri-list` (вид — `Copy`). Пустые строки и `\r` в конце строки допускаются; не-`file://` URI или неизвестная первая строка — весь буфер отвергается.

## Ядро: `clipboard.rs`

```rust
pub enum Kind { Copy, Cut }
pub struct Mime { pub gnome: String, pub uri_list: String, pub plain: String }
pub fn encode(kind: Kind, paths: &[PathBuf]) -> Mime;
/// `None` for an unknown mime type, a non-`file://` URI or a malformed header.
pub fn decode(mime: &str, data: &[u8]) -> Option<(Kind, Vec<PathBuf>)>;
```
URI через крейт `url` (`Url::from_file_path` / `to_file_path`); он уже в `Cargo.lock`.

## Приложение

- keymap: Ctrl + физическая `KeyC` / `KeyX` / `KeyV` → `Action::ClipCopy` / `ClipCut` / `ClipPaste` (работает на любой раскладке).
- `ClipCopy` / `ClipCut`: `targets()` активной панели → `clipboard::write_data(Files(encode(..)))`, где `Files` реализует `AsMimeTypes`. Нет целей — ничего.
- `ClipPaste`: если идёт операция — ничего; иначе `clipboard::read_data::<Paste>()` → `Message::Pasted(Option<(Kind, Vec<PathBuf>)>)`. `Paste` реализует `AllowedMimeTypes` + `TryFrom<(Vec<u8>, String)>` через `decode`.
- `Pasted(Some((kind, paths)))` с непустым `paths` → `start_transfer(Copy|Move, active, paths, "")`. Для `Cut` перед стартом — `clipboard::write(String::new())`.
- `Pasted(None)` или пустой список — ничего.

## Тесты

Core:
- round-trip `encode` → `decode` для `Copy` и `Cut` через `gnome`, и через `uri_list` (вид `Copy`);
- имена с пробелом, кириллицей, `%`, `#`;
- `plain` содержит пути через `\r\n`;
- `decode`: `http://` → `None`, первая строка `move` → `None`, неизвестный MIME → `None`, `\r\n` и пустые строки допустимы.

App: `Pasted(Cut, …)` запускает перенос в `cwd` активной панели; `Pasted(Copy, …)` — копирование; `Pasted(None)` ничего не делает; `ClipCopy` без целей (только `..`) ничего не пишет.

keymap: Ctrl+C/X/V, в том числе с кириллической раскладкой (`с`/`ч`/`м` на тех же физических клавишах).

Вручную (TESTING.md): копирование и вырезание из cosmic-files → Ctrl+V у нас, и обратно; Ctrl+C в поле быстрого поиска копирует текст.

## Вне фичи

Затемнение вырезанных файлов, вставка картинки как файла, вставка в тот же каталог как «(copy)», primary selection.
