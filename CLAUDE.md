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
sudo just install                # binary, .desktop, metainfo, icon into /usr
```

At startup libcosmic logs `error loading system dark theme ... GetKey("list_button")`. This is harmless: the pinned libcosmic expects theme keys that the system config doesn't have.

## Architecture

The workspace has two crates.

- **`crates/core` (`shagoff-core`)** has no libcosmic dependency. All logic lives here and is unit-tested.
  - `panel.rs`: `Panel { cwd, entries, cursor, marked, sort, show_hidden }` plus pure functions (cursor moves, Insert/Space marking, mask marking, invert, sort, `targets()`).
  - `listing.rs`: `scan()` on `std::fs`. `..` comes first, then folders, then files.
  - `archive.rs`: pack/unpack zip, tar.*, 7z through the same `Handler`. A path through an archive (`/x/a.zip/docs`) is a plain path: `listing::scan` falls back to `archive::list` (cached index) when `split_path` finds an archive; `extract` copies chosen entries out; the app refuses changes there (`App::read_only`). Unpack goes into a hidden `.shagoff-unpack.<pid>.<n>` in the destination (entry paths checked by `safe_path`, never through a symlink), then `ops::transfer(Move)`; pack writes a part file and renames without replacing.
  - `mount.rs`: mount / unmount through the `gio` CLI (udisks + gvfs, no glib in the build). Network mounts are dirs under `$XDG_RUNTIME_DIR/gvfs` (FUSE), so they are plain paths; gio's password prompts are answered by reading its stdout (once: it re-asks forever).
  - `ops.rs`: our own synchronous engine (`plan`, `transfer`, `delete`) driven through a `Handler` trait — not cosmic-files' (that one needs compio; see the phase 5 spec). Writes go to a hidden `.<name>.<pid>.<n>.shagoff-part` (created with O_EXCL) then `rename`; a moved source is deleted only after its copy succeeded; symlinks are never followed.
- **`crates/app` (`shagoff-commander`)** is the libcosmic UI.
  - `app.rs`: `App { panes: [Pane; 2], active }`.
  - tabs: each pane is a `core::tabs::Tabs<Tab>`, where `Tab` (in `app.rs`) = `core::Panel` + scroll state + pending scan + error.
  - `view/`: drive buttons, tabs, path line, column table, status line, F-key bar.
  - `keymap.rs`: one `KeyBind → Action` table with TC defaults. F-key buttons dispatch the same `Action`.
  - `drawer.rs` + `help.rs`: side drawer (libcosmic context drawer) with help (F1), about (libcosmic `about` feature; `DONATE_URL` for the future Stripe Payment Link) and settings (Ctrl+,). Settings write the whole `Config` and go through `App::apply_config`, the same path as a hand edit seen by the config watcher.
  - `menu.rs`: main menu in the header bar; items send the same `Action` as their key. Its key table (shown next to items) is checked against `keymap::action` by a test.
  - `dialogs.rs`: modal dialogs.
  - `watcher.rs`: `notify` on the active tab's cwd of each pane, debounced; ignores open/read events (our own scan opens the dir). Paused while a file operation runs.
  - `config.rs`: cosmic-config `Config` (settings, `~/.config/cosmic/<APP_ID>/v1/`, applied live) and `State` (tabs, `~/.local/state/cosmic/<APP_ID>/v1/`, written when it changes). `App::build` takes both, so tests never touch disk.

Data flow: key or button → keymap → `Action` → `App::update`. From there, one of:
- navigation or selection → a pure `core::Panel` function;
- directory change → `spawn_blocking(scan)` → `Message::Listed`;
- file operation → dialog → `ops` task → events → rescan both panes.

Implemented so far (MVP done): `crates/core` (`listing`, `sort`, `panel`, `format`, `viewport`, `tabs`, `mask`, `multirename`, `ops`, `archive`, `search`, `sync`, `diff`, `drives`, `mount`, `session`, `launch`); `crates/app` (`app.rs` two panes of `Tabs<Tab>` (scan results routed by tab id) + background scan with a generation check, `keymap.rs`, `view.rs` virtualized list). Marks and file operations (F5/F6/F7/F8, rename, progress/conflict/error dialogs) are in; `dialogs.rs` holds the `Dialog` enum + views, `jobs.rs` runs `ops` on a worker thread (events over a futures channel, answers over `std::sync::mpsc`, cancel = `AtomicBool`). `config.rs` and `watcher.rs` are in; drive buttons, F3/F4 and Alt+F1/F2 too. Modules appear phase by phase, so check the tree before assuming one exists.

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

Docs and skills live in `.claude/` and are committed (only `.claude/settings.local.json` is ignored). Lessons that are general to libcosmic go into the global `cosmic-applet` skill.
