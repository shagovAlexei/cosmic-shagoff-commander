# CLAUDE.md

Guidance for Claude Code in this repository. Reply to the user in Russian; code, identifiers and commit messages are in English.

## Project

Shagoff Commander is a dual-pane file manager for Pop!_OS 24.04 COSMIC, written in Rust on libcosmic (an iced fork). The goal is to behave as close as possible to **Total Commander for Windows** in layout and keyboard.

- `.claude/docs/tc-reference.md` is the source of truth for "how TC does it". Check it before implementing any key or behaviour, and update it when you add one.
- Full design and its rationale: `.claude/docs/specs/00-architecture-design.md`.
- Phases and backlog: `.claude/docs/ROADMAP.md`.

Fixed names (never change these):

| | |
|---|---|
| Repo | `shagovAlexei/cosmic-shagoff-commander` |
| Binary / crate | `shagoff-commander` |
| APP_ID | `io.github.shagovAlexei.cosmic-shagoff-commander` (in `crates/app/src/app.rs`, `res/*`, config path) |
| License | GPL-3.0-only (code adapted from cosmic-files is allowed; mark the source at the top of the file) |

## Commands

```bash
just verify                      # fmt --check + clippy -D warnings + test — must pass before every PR
just build-debug / build-release
just run-logs                    # RUST_LOG=debug cargo run
cargo run -p shagoff-commander   # run the app (default log level: warn)
cargo test -p shagoff-core <name>
sudo just install                # binary, .desktop, metainfo, icon, man page (res/shagoff-commander.1) into /usr (rootdir=… to stage)
just deb                         # target/deb/*.deb (dpkg-deb, deps by dpkg-shlibdeps); tag vX.Y.Z → release with it (built on Ubuntu 22.04: glibc ≥ 2.35)
just rpm                         # target/generate-rpm/*.rpm (cargo-generate-rpm, metadata in crates/app/Cargo.toml); in the release too
```

`main.rs` probes the GPU first: without `SHADER_F16_IN_F32` (old / software GL, e.g. X11 without DRI3) iced_wgpu panics on its shaders, so it sets `ICED_BACKEND=tiny-skia` (software renderer); a hand-set `ICED_BACKEND` wins. Without the "Cosmic" icon theme (outside COSMIC) it passes the desktop's icon theme (xfconf, gtk-3.0 `settings.ini`, gsettings) as `default_icon_theme`, else libcosmic's icons (title bar buttons, toolbar) come out blank.

At startup libcosmic logs `error loading system dark theme ... GetKey("list_button")`. This is harmless: the pinned libcosmic expects theme keys that the system config doesn't have.

## Architecture

The workspace has two crates.

- **`crates/core` (`shagoff-core`)** has no libcosmic dependency. All logic lives here and is unit-tested.
  - `panel.rs`: `Panel { cwd, entries, cursor, marked, sort, show_hidden }` plus pure functions (cursor moves, Insert/Space marking, mask marking, invert, sort, `targets()`).
  - `listing.rs`: `scan()` on `std::fs`. `..` comes first, then folders, then files.
  - `archive.rs`: pack/unpack zip, tar.*, 7z through the same `Handler`. A path through an archive (`/x/a.zip/docs`) is a plain path: `listing::scan` falls back to `archive::list` (cached index) when `split_path` finds an archive; `extract` copies chosen entries out; changes (F5/F6 in, out or between archives, F7, F8, Shift+F6, Ctrl+M, F4) go through `repack.rs`, the rest is refused (`App::read_only`). Unpack goes into a hidden `.shagoff-unpack.<pid>.<n>` in the destination (entry paths checked by `safe_path`, never through a symlink), then `ops::transfer(Move)`; pack writes a part file and renames without replacing. Passwords (zip AES / 7z): `pack_with`; reading takes `archive::password` (typed in `Dialog::Password` on `PASSWORD_ERROR`, kept until exit).
  - `repack.rs`: changing an archive = unpack it into a staging dir next to it, change the files, `pack` again in the same format, rename over the original (untouched until the new one is complete). `cross`: archive → archive = extract into a staging dir next to the target, `modify(Add)`, with F6 `modify(Delete)` on the source. F4 in an archive: `App::edited` + a 2 s tick offer to put a saved file back.
  - `mount.rs`: mount / unmount through the `gio` CLI (udisks + gvfs, no glib in the build). Network mounts are dirs under `$XDG_RUNTIME_DIR/gvfs` (FUSE), so they are plain paths; gio's password prompts are answered by reading its stdout (once: it re-asks forever). Ctrl+F can be cancelled (the gio process group is killed); addresses are saved without passwords; `browse` lists `network:///`.
  - `openwith.rs`: context menu «Open with…» — content type by `gio info`, programs by `gio mime` (default first), names and icons from the `.desktop` files, run by `gio launch`; the list is a `Dialog::List` (`ListKind::OpenWith`) ending in «Other program…» (`InputOp::OpenWith`, `sh -c` with the files appended); «Default» sets `gio mime TYPE app`; «All programs…» lists every `.desktop` (`all_apps`); typed commands are kept in `State.other_cmds`.
  - `keyring.rs`: Ctrl+F «Remember the password» — the item gvfs itself saves (`org.gnome.keyring.NetworkPassword`, `gvfs_attrs`: sftp / smb / ftp, not WebDAV whose lookup needs the server's realm), written over D-Bus (zbus, Secret Service, `plain` session) once connected; gvfs then never asks for it, from us or any program.
  - `owners.rs`: uid/gid → names from `/etc/passwd` / `/etc/group` (read once) for the status bar; `Entry.owner` / `Entry.target` are filled by `scan` so rendering never touches the disk.
  - `props.rs`: Alt+Enter — `usage` (size of a selection with contents, stoppable, links not followed) and `chgrp` to one of the user's groups (`owners::my_groups`, `lchown`, before the bits), `chmod` of only the toggled bits (recursive: dirs get added bits before and removed bits after their contents; links never touched).
  - `ops.rs`: our own synchronous engine (`plan`, `transfer`, `delete`) driven through a `Handler` trait — not cosmic-files' (that one needs compio; see the phase 5 spec). Writes go to a hidden `.<name>.<pid>.<n>.shagoff-part` (created with O_EXCL) then `rename`; a moved source is deleted only after its copy succeeded; symlinks are never followed. Conflicts: Replace / Skip (… all), Replace older, `Rename(name)` (asked again while the name is taken; `valid_name` keeps it in the dir), `RenameAll` (first free `unique_name`, `a (1).tar.gz`); copied dirs get their source's mtime after their contents.
- **`crates/app` (`shagoff-commander`)** is the libcosmic UI.
  - `app.rs`: `App { panes: [Pane; 2], active }`.
  - tabs: each pane is a `core::tabs::Tabs<Tab>`, where `Tab` (in `app.rs`) = `core::Panel` + scroll state + Brief view (Ctrl+F1, first column `col`; math in `core::viewport`) + pending scan + error + `locked` / `name` (saved in `PaneState`). A locked tab never leaves its dir: `App::load` opens a new tab for that.
  - `view.rs`: drive buttons, tabs, path line, column table or Brief columns (theme icon per row from `core::format::icon_name`). Two skins (`config::Skin`, Ctrl+, → Interface): Classic (toolbar, drive drop-down, header bar) and Modern (cards, breadcrumbs); the row height comes from the skin (`App::row_h()`), never a constant; `footer`: one status line (each pane's totals under it, the cursor entry details in the active half; a message — error, note, "working…" — and each pane's read error are cards of their own over the bottom of the panels, `status_popup`, whole and wrapped, each × closes its own), the command line (`path>`, `App::cmdline`; logic in `core::cmdline`: `cd`, quoting, history) and the F-key bar.
  - `keymap.rs`: one `KeyBind → Action` table with TC defaults. F-key buttons dispatch the same `Action`. The user's own keys (154, `hotkeys.rs`: Configuration → Hotkeys, `config.hotkeys` = ("Ctrl+K", `Action` Debug name or "" = off), `core::hotkey::Bind`) are a global (`keymap::set_own`, `route_event` has no app) tried first; letters by physical key; the menu shows them (`menu::key_binds`). While the dialog waits for a key, `keymap::CAPTURE` makes `route_event` hand the next one over.
  - `drawer.rs` + `help.rs`: side drawer (libcosmic context drawer) with help (F1), about (libcosmic `about` feature; `DONATE_URL`: the Payment Link «Shagoff Commander — Donate» of the separate Stripe account «Shagoff Commander», amount of the donor's choice in EUR) and settings (Ctrl+,). Settings write the whole `Config` and go through `App::apply_config`, the same path as a hand edit seen by the config watcher.
  - `thumbs.rs`: Thumbnails view (Ctrl+Shift+F1, `Tab.thumbs`, `view::thumb_list`): cell sizes and `make` (decoded by `lister::decode`, shrunk to 96 px). `App::want_thumbs`, run after every `update`, asks for the pictures on screen (at most `BUSY_MAX` at once); `App.thumbs` caches them by path + mtime.
  - `lister.rs`: F3 viewer (`App::lister`), by default in a window of its own (`Lister.window`, `window::open`, drawn by `view_window` → `view::lister_window` with its own `header_bar`; keys go to it when it is focused, `App::viewer()` via `core.focused_window()`), with `Config.lister_window` off in place of the panels; text / hex / image modes, search, N/P. Logic (line index, hex rows, search, mode detection) in `core::lister`. A file over `LIMIT` (32 MB) is held as a window (`Doc.start`, `core::lister::load_at`, from a line start, UTF-16 by the first window's BOM); scrolling to its edge, Home / End and search read another (`Lister::slide`, `Message::ListerPart`), the scroll kept on the same line (`line_row`). Its letter keys (`1 3 4 N P Q`, `W` wrap, `A S K 8` encodings 1251 / 866 / KOI8-R / UTF-8) come from `keymap::lister_key`, tried only after the main table found nothing. A file without BOM that is not UTF-8 opens in a guessed code page (`core::lister::guess`: `chardetng` tells the script, our own scorer picks the Russian page; `encoding_rs` decodes, `Encoding::Other` for the non-Cyrillic ones); a hand-picked encoding carries over N / P, a guess does not.
  - drag & drop: rows are `clip::drag` sources (inside an archive offered only as our own `INNER` mime type, Ctrl+C / Ctrl+X there too: no other program takes paths that are not on disk; dropped on us they go the F5 / F6 way) (window-bound; `dnd.rs` — libcosmic's `dnd_source` copied so the right button drags too: `Message::DragStart` keeps those files, their drop opens `Dialog::DropMenu` Copy / Move / Link at the drop point — DnD motion in `route_event` → `App::drop_at` (a zone's own `on_drop` point is shifted by its scrollable), a libcosmic `popover` with `Position::Point` inside the dialog layer), dir rows and the list `clip::drop_zone`s → `Message::Drop` → the F5 / F6 dialog; the right-click `context_menu` and the size `sensor` sit in a `stack` layer under the list, because neither passes drop zones (`drag_destinations`) through.
  - right button (105): with `Config.right_marks` (default) a quick right click marks (`Message::RightUp` from `route_event`), held 500 ms (`App::right_press` timer → `RightHeld`) it opens the menu; `catch.rs` keeps a quick click's release from libcosmic's `context_menu` (it opens on the release and cannot be given no menu: its `diff` unwraps it).
  - `menu.rs`: main menu in the header bar and the right-click menus (`menu::context(Ctx)` for a row, the dir, a tab); items send the same `Action` as their key. Its key table (shown next to items) is checked against `keymap::action` by a test. Context menus are Wayland popups (`on_surface_action` → `Message::Surface`); `on_close_requested` must only exit for the main window, popups close through it too.
  - `dialogs.rs`: modal dialogs; `menu_row` draws a TC-menu-like row (Ctrl+D, Alt+F1/F2, Alt+↓). `hotlist.rs`: Ctrl+D → «Настроить…» (`HotEdit` edits a copy of `config.hotlist`, saved on OK; a separator is `-` with no path). `toolbar.rs`: the classic skin's button bar from `config.toolbar` (`cm_*` TC command names → `Action` in `builtins()`, or a program run by `sh -c` with `core::cmdline::expand` parameters); right click → `ToolEdit` dialog. F12 runs `config.f12`, a `ToolButton` set in the settings, through the same `App::run_button`.
  - `watcher.rs`: `notify` on the active tab's cwd of each pane, debounced; ignores open/read events (our own scan opens the dir). Paused while a file operation runs.
  - `term/`: built-in terminal (149), adapted from cosmic-term (`terminal.rs`, `terminal_box.rs`, `terminal_theme.rs`, `mouse_reporter.rs`; `alacritty_terminal` + `cosmic-text`): one terminal, `style::Style` in place of cosmic-term's config. Under the panels in place of the command line (`view::footer`), Ctrl+O in place of the panels (`main_view`); events through `term::subscription` (`TermTx` / `TermEvent`). Its shell starts once the active panel's dir is read and follows it with ` cd -- dir` only at its prompt (`core::shell::at_prompt`: holds the foreground and, with a line editor, the tty in raw mode). The box takes every key while focused but `term::pass` (Ctrl+O); a click on a pane unfocuses it (`App::term_blur`), the window's focus coming back restores it. External terminals: `core::terminals` (installed `.desktop` `TerminalEmulator`, the system default, each one's run flag), Commands → Terminal… / Open terminal.
  - `config.rs`: cosmic-config `Config` (settings, `~/.config/cosmic/<APP_ID>/v1/`, applied live) and `State` (tabs, `~/.local/state/cosmic/<APP_ID>/v1/`, written when it changes). `App::build` takes both, so tests never touch disk.

Data flow: key or button → keymap → `Action` → `App::update`. From there, one of:
- navigation or selection → a pure `core::Panel` function;
- directory change → `spawn_blocking(scan)` → `Message::Listed`;
- file operation → dialog → `ops` task → events → rescan both panes.

Implemented so far (MVP done): `crates/core` (`listing`, `sort`, `panel`, `format`, `viewport`, `tabs`, `mask`, `multirename`, `ops`, `archive`, `repack`, `search`, `sync`, `diff`, `drives`, `mount`, `openwith`, `owners`, `session`, `launch`, `cmdline`, `clipboard`, `history`, `quicksearch`, `lister`, `props`); `crates/app` (`app.rs` two panes of `Tabs<Tab>` (scan results routed by tab id) + background scan with a generation check, `keymap.rs`, `view.rs` virtualized list). Marks and file operations (F5/F6/F7/F8, rename, progress/conflict/error dialogs) are in; `dialogs.rs` holds the `Dialog` enum + views, `jobs.rs` runs `ops` on a worker thread (events over a futures channel, answers over `std::sync::mpsc`, cancel = `AtomicBool`). One job at a time (`App::job`); "In background" (`Running.hidden`) hides its dialog and shows it above the status line — `App::busy()` (a job dialog is up) is what blocks the panels, not `job.is_some()`. `config.rs` and `watcher.rs` are in; drive buttons, F3/F4 and Alt+F1/F2 too. Modules appear phase by phase, so check the tree before assuming one exists.

## Conventions

- **TC semantics:** the cursor and the marks are separate. Operations apply to the marked files, or to the file under the cursor if nothing is marked (`Panel::targets()`). `..` can never be marked.
- **Thin `update()`:** logic goes into `crates/core` as pure functions with tests. Don't put logic in view or update arms.
- **libcosmic** is pinned by `rev` in `crates/app/Cargo.toml` (pre-1.0, it breaks without warning). Bump it only in its own PR. Pinned features: `tokio, winit, wayland, wgpu, multi-window`.
- **Config:** a `#[derive(CosmicConfigEntry)]` struct with `#[version = 1]`. Every field has a `Default` so old installs keep loading. Read with `get_entry(...).unwrap_or_else(|(_, c)| c)`.
- **i18n:** Fluent via `fl!("key")` (checked at compile time) in `crates/app/i18n/{en,ru}/shagoff-commander.ftl`. Every new string needs both `en` and `ru`.
- Edition 2024. `Cargo.lock` is committed and CI builds with `--locked`.

## Workflow

Every feature or phase follows the project skill **`shagoff-feature`** (`.claude/skills/shagoff-feature/SKILL.md`):

1. brainstorm → spec in `.claude/docs/specs/`
2. `writing-plans` → plan in `.claude/docs/plans/NN-<topic>.md`
3. branch `feat/<topic>`, TDD in core
4. `just verify` + manual run
5. `/code-review`
6. PR via `gh`, green CI, merge only after the user approves

Never commit to `main` directly. Bugs get a `regression_<what>` test and a row in `TESTING.md`.

**Versions:** every PR that changes the program bumps `version` in the root `Cargo.toml` (and `Cargo.lock`) in the same PR: a feature → minor (0.2.0 → 0.3.0), a bug fix → patch (0.2.0 → 0.2.1); docs / tests / tooling only → no bump. After the user approves the merge: `git tag -a vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z` → the `release` workflow publishes the .deb.

**README:** two files kept in step: `README.md` (English) and `README_ru.md` (Russian), each linking to the other at the top. A change to one goes into the other in the same PR.

Docs and skills live in `.claude/` and are committed (only `.claude/settings.local.json` is ignored). Lessons that are general to libcosmic go into the global `cosmic-applet` skill.
