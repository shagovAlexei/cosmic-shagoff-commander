# Phase 6: F3/F4, Drives, Watcher, Config — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the MVP: F3/F4 through external programs, drive buttons with free space and Alt+F1/F2, auto-refresh on directory changes, cosmic-config settings plus remembered tabs, Ctrl+H, and a configurable Ctrl+W on the last tab.

**Architecture:** The pure parts go in `crates/core` and are unit-tested: mount parsing, session restore, launch argv, human sizes. `crates/app` gets `config.rs` (two `CosmicConfigEntry` structs) and `watcher.rs` (a notify subscription), and `App` gains a `build()` constructor that takes config and state, so tests run without touching disk.

**Tech Stack:** Rust 2024, libcosmic rev `ef490df50b0a05a21c494c3f75737581bf0b39d9` (cosmic-config, iced `Subscription::run_with`, `iced::stream::channel`), notify 8.2, rustix 1 (`fs`), serde 1.

**Spec:** `.claude/docs/specs/06-mvp-finish.md`

## Global Constraints

- APP_ID: `io.github.shagovAlexei.cosmic-shagoff-commander`, also used as the cosmic-config id for both `Config` and `State`. Both are `#[version = 1]`.
- Every config field has a `Default`. Read with `get_entry(&h).unwrap_or_else(|(_, c)| c)`.
- No new crates beyond what `Cargo.lock` already has. CI builds with `--locked`; after you add deps, run `cargo build` once and commit the lock.
  - `notify = "8.2"`, `serde` (derive) and `rustix` (`fs`) are already in the lock. Add them as direct deps.
  - Add `tokio` feature `time` (already compiled via cosmic-config).
- `crates/core` must not depend on libcosmic.
- Every new UI string goes into both `en` and `ru` ftl.
- Letter shortcuts match physical keys (`Code::KeyH`) so they work in the Russian layout. Modifiers must match exactly.
- Keys are never sent to the desktop with `wtype`. Don't steal focus for screenshots.
- `just verify` passes before every commit that closes a task.

## Review Focus

1. **Self-triggered refresh loop.** notify's inotify backend subscribes to `IN_OPEN`, and our own scan opens the watched dir. If `Access` events aren't filtered, the panel rescans forever. Pinned by `watcher::tests::open_and_read_events_are_ignored` (Task 8).
2. **Stale or garbage state file.** Deleted dirs, relative paths, `active` out of range, or an empty tab list must still start the app in a sane dir. Pinned by the `session` tests (Task 2).
3. **Same device mounted twice, or names with spaces in mounts** (`\040`). The user expects one button with a readable label. Pinned by the `drives::parse` tests (Task 1).
4. **Two scans of the same path in flight** (Ctrl+R twice, watcher + Ctrl+R). The newest result must win. Pinned by `regression_second_scan_of_same_path_wins` (Task 5).
5. **Alt rule.** Only Alt+F1 and Alt+F2 pass. Alt+Enter, Alt+letters and Alt+Shift+F1 still give `None`, so the compositor/menu keys keep working. Pinned by `keymap::tests::alt_only_f1_f2` (Task 3).

---

### Task 1: core `drives` + `format::human`

**Files:**
- Create: `crates/core/src/drives.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod drives;`), `crates/core/src/format.rs` (add `human`), `crates/core/Cargo.toml` (rustix)

**Interfaces:**
- Produces:
  - `drives::Drive { pub label: String, pub path: PathBuf }` (Clone, Debug, PartialEq, Eq);
  - `drives::parse(mounts: &str, home: &Path) -> Vec<Drive>`;
  - `drives::containing(drives: &[Drive], path: &Path) -> Option<usize>`;
  - `drives::space(path: &Path) -> Option<(u64, u64)>`, returning (free, total) in bytes;
  - `format::human(n: u64) -> String`.

- [ ] **Step 1: Add rustix to core**

`crates/core/Cargo.toml` `[dependencies]`:
```toml
rustix = { version = "1", features = ["fs"] }
```

- [ ] **Step 2: Write the failing tests** (bottom of new `crates/core/src/drives.rs`, with stub fns that `todo!()`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from this machine's /proc/self/mounts.
    const MOUNTS: &str = "\
sysfs /sys sysfs rw,nosuid 0 0
/dev/nvme1n1p2 / ext4 rw,noatime 0 0
/dev/loop2 /snap/bare/5 squashfs ro 0 0
/dev/loop0 /snap/acestreamplayer/18 squashfs ro 0 0
/dev/nvme1n1p1 /boot/efi vfat rw 0 0
/dev/nvme1n1p4 /home ext4 rw 0 0
/dev/sda1 /mnt/save-flash-home vfat rw 0 0
/dev/sda1 /media/shag/SAVE_FLASH vfat rw 0 0
/dev/nvme0n1p3 /media/shag/sys fuseblk rw 0 0
tmpfs /run tmpfs rw 0 0
";

    fn labels(d: &[Drive]) -> Vec<&str> {
        d.iter().map(|d| d.label.as_str()).collect()
    }

    #[test]
    fn parse_keeps_root_home_and_real_devices() {
        let d = parse(MOUNTS, Path::new("/home/shag"));
        assert_eq!(labels(&d), ["/", "~", "home", "SAVE_FLASH", "sys"]);
        assert_eq!(d[1].path, Path::new("/home/shag"));
        assert_eq!(d[3].path, Path::new("/media/shag/SAVE_FLASH"));
    }

    #[test]
    fn parse_prefers_media_mount_for_a_device_mounted_twice() {
        let m = "/dev/sdb1 /media/u/X vfat rw 0 0\n/dev/sdb1 /mnt/x vfat rw 0 0\n";
        let d = parse(m, Path::new("/home/u"));
        assert_eq!(labels(&d), ["/", "~", "X"]);
        let m = "/dev/sdb1 /mnt/x vfat rw 0 0\n/dev/sdb1 /run/media/u/X vfat rw 0 0\n";
        let d = parse(m, Path::new("/home/u"));
        assert_eq!(d[2].path, Path::new("/run/media/u/X"));
    }

    #[test]
    fn parse_unescapes_spaces() {
        let d = parse("/dev/sdb1 /media/u/My\\040Disk vfat rw 0 0\n", Path::new("/home/u"));
        assert_eq!(d[2].label, "My Disk");
        assert_eq!(d[2].path, Path::new("/media/u/My Disk"));
    }

    #[test]
    fn parse_skips_root_and_home_repeats() {
        let m = "/dev/a / ext4 rw 0 0\n/dev/b /home/u ext4 rw 0 0\n";
        assert_eq!(labels(&parse(m, Path::new("/home/u"))), ["/", "~"]);
    }

    #[test]
    fn containing_picks_longest_prefix_by_component() {
        let d = parse(MOUNTS, Path::new("/home/shag"));
        let label = |p: &str| containing(&d, Path::new(p)).map(|i| d[i].label.as_str());
        assert_eq!(label("/home/shag/x"), Some("~"));
        assert_eq!(label("/home/other"), Some("home"));
        assert_eq!(label("/media/shag/sys/a"), Some("sys"));
        assert_eq!(label("/etc"), Some("/"));
        assert_eq!(label("/homework"), Some("/"));
    }

    #[test]
    fn space_of_root_is_sane() {
        let (free, total) = space(Path::new("/")).expect("statvfs /");
        assert!(total > 0 && free <= total);
    }
}
```

In `crates/core/src/format.rs` tests module add:
```rust
#[test]
fn human_sizes() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(1023), "1023 B");
    assert_eq!(human(1536), "1.5 K");
    assert_eq!(human(12_900_000_000), "12.0 G");
    assert_eq!(human(450 * 1024 * 1024 * 1024), "450 G");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p shagoff-core drives format::tests::human_sizes`
Expected: FAIL (panics on `todo!()` / `human` not found).

- [ ] **Step 4: Implement**

`crates/core/src/drives.rs` (above the tests):
```rust
//! Drive buttons: `/`, `~` and real block devices from `/proc/self/mounts`.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub label: String,
    pub path: PathBuf,
}

/// `/` and `home` first, then `/dev/*` mounts except loop/squashfs/`/boot*`, in mounts order.
/// A device mounted twice gives one drive: its `/media` or `/run/media` mount, else the first.
pub fn parse(mounts: &str, home: &Path) -> Vec<Drive> {
    let mut out = vec![
        Drive { label: "/".into(), path: "/".into() },
        Drive { label: "~".into(), path: home.to_path_buf() },
    ];
    let mut devices: Vec<&str> = Vec::new(); // devices[i] owns out[i + 2]
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(point), Some(fs)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if !dev.starts_with("/dev/") || dev.starts_with("/dev/loop") || fs == "squashfs" {
            continue;
        }
        let path = unescape(point);
        if path == Path::new("/") || path == home || path.starts_with("/boot") {
            continue;
        }
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let drive = Drive { label, path };
        match devices.iter().position(|d| *d == dev) {
            Some(i) => {
                if is_media(&drive.path) && !is_media(&out[i + 2].path) {
                    out[i + 2] = drive;
                }
            }
            None => {
                devices.push(dev);
                out.push(drive);
            }
        }
    }
    out
}

fn is_media(p: &Path) -> bool {
    p.starts_with("/media") || p.starts_with("/run/media")
}

/// The kernel escapes space, tab, newline and backslash in mounts as `\ooo` octal.
fn unescape(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(byte) = s.get(i + 1..i + 4).and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    OsString::from_vec(out).into()
}

/// Index of the drive whose path is the longest (component-wise) prefix of `path`.
pub fn containing(drives: &[Drive], path: &Path) -> Option<usize> {
    drives
        .iter()
        .enumerate()
        .filter(|(_, d)| path.starts_with(&d.path))
        .max_by_key(|(_, d)| d.path.components().count())
        .map(|(i, _)| i)
}

/// (free for unprivileged users, total) bytes of the filesystem holding `path`.
pub fn space(path: &Path) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some((s.f_bavail * s.f_frsize, s.f_blocks * s.f_frsize))
}
```

`crates/core/src/format.rs`:
```rust
/// `1536` → `1.5 K`; binary units, one decimal below 100.
pub fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["K", "M", "G", "T", "P"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v < 100.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}
```
`lib.rs`: add `pub mod drives;` (alphabetical).

- [ ] **Step 5: Run tests**

Run: `cargo test -p shagoff-core drives format`
Expected: PASS. If `human(12_900_000_000)` is off by rounding, fix the assertion to the computed value (12_900_000_000 / 1024³ = 12.01 → `12.0 G`).

- [ ] **Step 6: Commit**

```bash
cargo build -q && git add crates/core Cargo.lock && git commit -m "core: drives from mounts, free space, human sizes"
```

---

### Task 2: core `session` + `launch`

**Files:**
- Create: `crates/core/src/session.rs`, `crates/core/src/launch.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/Cargo.toml` (serde)

**Interfaces:**
- Produces:
  - `session::PaneState { pub tabs: Vec<PathBuf>, pub active: usize }` (Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize);
  - `session::existing_dir(path: &Path, fallback: &Path) -> PathBuf`;
  - `session::restore(state: &PaneState, fallback: &Path) -> (Vec<PathBuf>, usize)`;
  - `launch::command(cmd: &[String], default: &[&str], file: &Path) -> Vec<OsString>`.

- [ ] **Step 1: serde dep**

`crates/core/Cargo.toml`: `serde = { version = "1", features = ["derive"] }`

- [ ] **Step 2: Write failing tests** (stubs with `todo!()`)

`session.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_dir_walks_up_to_a_live_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("a/b/c");
        assert_eq!(existing_dir(&gone, Path::new("/fallback")), tmp.path());
        assert_eq!(existing_dir(tmp.path(), Path::new("/fallback")), tmp.path());
    }

    #[test]
    fn existing_dir_skips_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(existing_dir(&file, Path::new("/fallback")), tmp.path());
    }

    #[test]
    fn existing_dir_relative_or_empty_gives_fallback() {
        assert_eq!(existing_dir(Path::new("rel/dir"), Path::new("/fb")), Path::new("/fb"));
        assert_eq!(existing_dir(Path::new(""), Path::new("/fb")), Path::new("/fb"));
    }

    #[test]
    fn restore_empty_state_opens_fallback() {
        let (tabs, active) = restore(&PaneState::default(), Path::new("/fb"));
        assert_eq!(tabs, [PathBuf::from("/fb")]);
        assert_eq!(active, 0);
    }

    #[test]
    fn restore_clamps_active_and_fixes_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let state = PaneState {
            tabs: vec![tmp.path().into(), tmp.path().join("gone"), "rel".into()],
            active: 7,
        };
        let (tabs, active) = restore(&state, Path::new("/fb"));
        assert_eq!(tabs, [tmp.path().to_path_buf(), tmp.path().into(), "/fb".into()]);
        assert_eq!(active, 2);
    }
}
```

`launch.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_uses_default() {
        let argv = command(&[], &["xdg-open"], Path::new("/a b/f.txt"));
        assert_eq!(argv, [OsString::from("xdg-open"), "/a b/f.txt".into()]);
    }

    #[test]
    fn configured_command_keeps_its_args_and_appends_file() {
        let cmd = vec!["foot".to_string(), "-e".into(), "less".into()];
        let argv = command(&cmd, &["xdg-open"], Path::new("/f"));
        assert_eq!(argv, ["foot", "-e", "less", "/f"].map(OsString::from));
    }
}
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo test -p shagoff-core session launch`
Expected: FAIL (`todo!()` panics).

- [ ] **Step 4: Implement**

`session.rs`:
```rust
//! Remembered tabs: what is saved per pane and how it is restored when dirs have gone.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneState {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
}

/// `path` or its nearest existing ancestor dir; `fallback` for relative or empty paths.
pub fn existing_dir(path: &Path, fallback: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| p.is_absolute() && p.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// Tab paths to open and the active index; never empty.
pub fn restore(state: &PaneState, fallback: &Path) -> (Vec<PathBuf>, usize) {
    if state.tabs.is_empty() {
        return (vec![fallback.to_path_buf()], 0);
    }
    let tabs: Vec<PathBuf> = state.tabs.iter().map(|p| existing_dir(p, fallback)).collect();
    let active = state.active.min(tabs.len() - 1);
    (tabs, active)
}
```

`launch.rs`:
```rust
//! argv for F3/F4: a configured program (no shell) with the file appended.

use std::ffi::OsString;
use std::path::Path;

pub fn command(cmd: &[String], default: &[&str], file: &Path) -> Vec<OsString> {
    let mut argv: Vec<OsString> = if cmd.is_empty() {
        default.iter().map(OsString::from).collect()
    } else {
        cmd.iter().map(OsString::from).collect()
    };
    argv.push(file.into());
    argv
}
```
`lib.rs`: `pub mod launch;` and `pub mod session;`.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo test -p shagoff-core session launch`

- [ ] **Step 6: Commit**

```bash
cargo build -q && git add crates/core Cargo.lock && git commit -m "core: session restore and launch argv"
```

---

### Task 3: keymap — F3, F4, Ctrl+H, Alt+F1/F2; drop `Action::Cancel`

**Files:**
- Modify: `crates/app/src/keymap.rs`, `crates/app/src/app.rs` (match arms only)

**Interfaces:**
- Produces: `Action::View`, `Action::Edit`, `Action::ToggleHidden`, `Action::Drives(usize)` (0 = left, 1 = right). `Action::Cancel` is removed.

- [ ] **Step 1: Failing tests** (add to `keymap.rs` tests; `ALT` = `Modifiers::ALT`)

```rust
const ALT: Modifiers = Modifiers::ALT;

#[test]
fn view_edit_hidden() {
    assert_eq!(named(Named::F3, NONE), Some(Action::View));
    assert_eq!(named(Named::F4, NONE), Some(Action::Edit));
    assert_eq!(chr("h", Code::KeyH, CTRL), Some(Action::ToggleHidden));
    assert_eq!(chr("р", Code::KeyH, CTRL), Some(Action::ToggleHidden));
    assert_eq!(chr("h", Code::KeyH, NONE), None);
}

#[test]
fn alt_only_f1_f2() {
    assert_eq!(named(Named::F1, ALT), Some(Action::Drives(0)));
    assert_eq!(named(Named::F2, ALT), Some(Action::Drives(1)));
    assert_eq!(named(Named::F1, ALT | Modifiers::SHIFT), None);
    assert_eq!(named(Named::Enter, ALT), None);
    assert_eq!(named(Named::F4, ALT), None); // Alt+F4 stays with the compositor
    assert_eq!(chr("a", Code::KeyA, ALT), None);
    assert_eq!(named(Named::F1, Modifiers::LOGO), None);
}

#[test]
fn escape_is_not_a_panel_key() {
    // Escape is routed to DialogCancel by app::route_event before the keymap.
    assert_eq!(named(Named::Escape, NONE), None);
}
```
If an existing test asserts `Action::Cancel`, delete that assertion.

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p shagoff-commander keymap`
Expected: compile error (`no variant View`).

- [ ] **Step 3: Implement**

In `enum Action`: remove `Cancel`; add `View, Edit, ToggleHidden, Drives(usize)`.

Replace the top of `action()`:
```rust
    if mods.logo() {
        return None;
    }
    if mods.alt() {
        // Alt+F1/F2 only; other Alt combos belong to the compositor and future menus.
        return match key {
            Key::Named(Named::F1) if mods == Modifiers::ALT => Some(Action::Drives(0)),
            Key::Named(Named::F2) if mods == Modifiers::ALT => Some(Action::Drives(1)),
            _ => None,
        };
    }
```
(`use cosmic::iced::keyboard::key::{Code, Named};` is already at the top of the fn; keep it before this block.)

In the named table: delete the `(Named::Escape, false, false) => Action::Cancel,` row; add
```rust
            (Named::F3, false, false) => Action::View,
            (Named::F4, false, false) => Action::Edit,
```
In the physical table add
```rust
        (Physical::Code(Code::KeyH), true, false) => Some(Action::ToggleHidden),
```

`app.rs` `act()`: delete `Action::Cancel => {}` and add a temporary arm, which later tasks replace:
```rust
            Action::View | Action::Edit | Action::ToggleHidden | Action::Drives(_) => {}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander keymap && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "keymap: F3, F4, Ctrl+H, Alt+F1/F2; drop unused Cancel"
```

---

### Task 4: config + remembered tabs + Ctrl+H + Ctrl+W on the last tab

**Files:**
- Create: `crates/app/src/config.rs`
- Modify: `crates/app/src/main.rs` (`mod config;`), `crates/app/src/app.rs`, `crates/app/Cargo.toml` (serde)

**Interfaces:**
- Consumes: `session::{PaneState, restore}` (Task 2); `Action::ToggleHidden` (Task 3).
- Produces:
  - `config::Config { show_hidden: bool, viewer: Vec<String>, editor: Vec<String>, last_tab_close: LastTab, home_dir: Option<PathBuf> }`;
  - `config::LastTab { Nothing, Home }`;
  - `config::State { panes: [PaneState; 2], active: usize }`;
  - `App::build(core: Core, config: Config, state: State, left: Option<PathBuf>, home: PathBuf) -> (App, Task<Message>)`, which never touches config files;
  - `App::update` = `handle(message)` + `save_state()`;
  - `App.home: PathBuf`, `App.config: Config`;
  - `Message::Config(Config)`;
  - `App::new_tab(&mut self, path: PathBuf) -> Tab`, which applies `config.show_hidden`;
  - `App::load_all(&mut self) -> Task<Message>`, which rescans every tab of both panes.

- [ ] **Step 1: `config.rs`**

`crates/app/Cargo.toml`: `serde = { version = "1", features = ["derive"] }`.

```rust
//! Settings (edited by hand, applied live) and state (written by the app): both cosmic-config.

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use shagoff_core::session::PaneState;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastTab {
    /// Ctrl+W on the last tab does nothing.
    #[default]
    Nothing,
    /// Ctrl+W on the last tab goes to `home_dir` (or `~`).
    Home,
}

/// `~/.config/cosmic/<APP_ID>/v1/<field>`.
#[derive(Clone, Debug, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct Config {
    pub show_hidden: bool,
    /// F3 program + args; empty → `xdg-open`.
    pub viewer: Vec<String>,
    /// F4 program + args; empty → `cosmic-edit`.
    pub editor: Vec<String>,
    pub last_tab_close: LastTab,
    pub home_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: false,
            viewer: Vec::new(),
            editor: vec!["cosmic-edit".into()],
            last_tab_close: LastTab::Nothing,
            home_dir: None,
        }
    }
}

/// `~/.local/state/cosmic/<APP_ID>/v1/<field>`.
#[derive(Clone, Debug, Default, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct State {
    pub panes: [PaneState; 2],
    pub active: usize,
}

pub fn config_handler() -> Option<cosmic_config::Config> {
    cosmic_config::Config::new(crate::app::APP_ID, Config::VERSION)
        .inspect_err(|e| log::warn!("config: {e}"))
        .ok()
}

pub fn state_handler() -> Option<cosmic_config::Config> {
    cosmic_config::Config::new_state(crate::app::APP_ID, State::VERSION)
        .inspect_err(|e| log::warn!("state: {e}"))
        .ok()
}

/// Stored value, or the default for anything missing or unreadable.
pub fn read<T: CosmicConfigEntry>(h: Option<&cosmic_config::Config>) -> T
where
    T: Default,
{
    h.map(|h| T::get_entry(h).unwrap_or_else(|(_, c)| c))
        .unwrap_or_default()
}
```
If the derive macro requires `Eq` or `Serialize` on the struct, add only what the compiler asks for and ledger it.

- [ ] **Step 2: Failing app tests** (add to `app.rs` `mod tests`)

```rust
use crate::config::{Config, LastTab, State};
use shagoff_core::session::PaneState;

fn app_with(config: Config, state: State) -> App {
    App::build(Core::default(), config, state, None, std::env::temp_dir()).0
}

fn cwds(app: &App, side: usize) -> Vec<PathBuf> {
    app.panes[side].items().iter().map(|t| t.panel.cwd().to_path_buf()).collect()
}

#[test]
fn build_restores_tabs_and_active_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    std::fs::create_dir(&a).unwrap();
    let state = State {
        panes: [
            PaneState { tabs: vec![tmp.path().into(), a.clone()], active: 1 },
            PaneState { tabs: vec![tmp.path().join("gone")], active: 0 },
        ],
        active: 1,
    };
    let app = app_with(Config::default(), state);
    assert_eq!(cwds(&app, 0), [tmp.path().to_path_buf(), a]);
    assert_eq!(app.panes[0].active_index(), 1);
    assert_eq!(cwds(&app, 1), [tmp.path().to_path_buf()]);
    assert_eq!(app.active, 1);
}

#[test]
fn argv_path_replaces_the_active_left_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let state = State {
        panes: [PaneState { tabs: vec!["/".into(), "/".into()], active: 1 }, PaneState::default()],
        active: 0,
    };
    let app = App::build(Core::default(), Config::default(), state, Some(tmp.path().into()), "/".into()).0;
    assert_eq!(cwds(&app, 0), [PathBuf::from("/"), tmp.path().canonicalize().unwrap()]);
}

#[test]
fn ctrl_h_flips_hidden_in_every_tab() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::NewTab));
    let _ = app.update(Message::Key(Action::ToggleHidden));
    assert!(app.config.show_hidden);
    for side in 0..2 {
        assert!(app.panes[side].items().iter().all(|t| t.panel.show_hidden()));
    }
}

#[test]
fn ctrl_w_on_last_tab_obeys_config() {
    let mut app = app_with(Config::default(), State::default());
    let before = cwds(&app, 0);
    let _ = app.update(Message::Key(Action::CloseTab));
    assert_eq!(cwds(&app, 0), before); // Nothing: no change
    assert!(app.panes[0].active().pending.is_none());

    let config = Config { last_tab_close: LastTab::Home, home_dir: Some("/".into()), ..Config::default() };
    let mut app = app_with(config, State::default());
    let _ = app.update(Message::Key(Action::CloseTab));
    assert!(app.panes[0].active().pending.is_some(), "should load home_dir");
}

#[test]
fn config_change_of_show_hidden_applies_to_tabs() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Config(Config { show_hidden: true, ..Config::default() }));
    assert!(app.panes[1].active().panel.show_hidden());
}
```
`crates/app/Cargo.toml` `[dev-dependencies]`: `tempfile = "3"` (already in lock via core).

The `Home` test asserts `pending.is_some()`, which works with `pending: Option<PathBuf>` today and `Option<u64>` after Task 5. Make `pending` `pub(crate)`.

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo test -p shagoff-commander app::tests`
Expected: compile errors (`no function build`, `no field config`).

- [ ] **Step 4: Implement in `app.rs`**

New fields on `App`:
```rust
    pub home: PathBuf,
    pub config: Config,
    config_handler: Option<cosmic_config::Config>,
    state_handler: Option<cosmic_config::Config>,
    /// Last state written, to skip identical writes.
    saved: State,
```
Imports: `use crate::config::{self, Config, LastTab, State};`, `use cosmic::cosmic_config::{self, CosmicConfigEntry};`, `use shagoff_core::session::{self, PaneState};`.

`init` becomes:
```rust
    fn init(mut core: Core, flags: Flags) -> (Self, Task<Message>) {
        core.window.header_title = fl!("app-title");
        // Tab is ours (switch pane); libcosmic's Tab focus-walk would also focus buttons that Enter then fires.
        core.set_keyboard_nav(false);
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let (ch, sh) = (config::config_handler(), config::state_handler());
        let (cfg, state) = (config::read(ch.as_ref()), config::read(sh.as_ref()));
        let (mut app, task) = Self::build(core, cfg, state, flags.left, home);
        app.saved = config::read(sh.as_ref());
        (app.config_handler, app.state_handler) = (ch, sh);
        (app, task)
    }
```
`build`:
```rust
    /// Everything but the disk-backed config handlers (tests use this directly).
    pub fn build(core: Core, config: Config, state: State, left: Option<PathBuf>, home: PathBuf) -> (Self, Task<Message>) {
        let mut app = Self {
            core,
            panes: [Tabs::new(Tab::new(0, home.clone())), Tabs::new(Tab::new(0, home.clone()))],
            scroll_ids: [widget::Id::unique(), widget::Id::unique()],
            active: state.active.min(1),
            tz: TimeZone::system(),
            next_id: 0,
            mods: Modifiers::empty(),
            dialog: None,
            job: None,
            input_id: widget::Id::unique(),
            home,
            config,
            config_handler: None,
            state_handler: None,
            saved: State::default(),
        };
        let left = left.and_then(|p| p.canonicalize().ok()).filter(|p| p.is_dir());
        for side in 0..2 {
            let (mut paths, active) = session::restore(&state.panes[side], &app.home);
            if side == 0 && let Some(l) = &left {
                paths[active] = l.clone();
            }
            let mut tabs = Tabs::new(app.new_tab(paths[0].clone()));
            for p in &paths[1..] {
                tabs.open_after(app.new_tab(p.clone()));
            }
            tabs.select(active);
            app.panes[side] = tabs;
        }
        let task = app.load_all();
        (app, task)
    }

    fn new_tab(&mut self, path: PathBuf) -> Tab {
        let mut t = Tab::new(self.next_id(), path);
        t.panel.set_show_hidden(self.config.show_hidden);
        t
    }

    /// Rescan every tab of both panes (startup, Ctrl+H).
    fn load_all(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for side in 0..2 {
            for i in 0..self.panes[side].items().len() {
                let cwd = self.panes[side].items()[i].panel.cwd().to_path_buf();
                tasks.push(self.load_tab(side, i, cwd, None));
            }
        }
        Task::batch(tasks)
    }
```
Refactor `load` into `load_tab(side, i, path, focus)`, which uses `&mut self.panes[side].items_mut()[i]` instead of `active_mut()`. Then `load(side, path, focus)` = `self.load_tab(side, self.panes[side].active_index(), path, focus)`.

Update: rename the current `fn update` body to `fn handle(&mut self, message: Message) -> Task<Message>` (in `impl App`), and make the trait method
```rust
    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.save_state();
        task
    }
```
```rust
    /// Write tab paths when they changed (cheap compare on every message; no write if equal).
    fn save_state(&mut self) {
        let state = State {
            panes: [0, 1].map(|s| PaneState {
                tabs: self.panes[s].items().iter().map(|t| t.panel.cwd().to_path_buf()).collect(),
                active: self.panes[s].active_index(),
            }),
            active: self.active,
        };
        if state == self.saved {
            return;
        }
        if let Some(h) = &self.state_handler
            && let Err(e) = state.write_entry(h)
        {
            log::warn!("state: {e}");
        }
        self.saved = state;
    }
```
`Message::Config(Config)` arm in `handle`:
```rust
            Message::Config(c) => {
                let hidden_changed = c.show_hidden != self.config.show_hidden;
                self.config = c;
                if hidden_changed {
                    return self.apply_hidden();
                }
            }
```
```rust
    fn apply_hidden(&mut self) -> Task<Message> {
        let on = self.config.show_hidden;
        for side in 0..2 {
            for t in self.panes[side].items_mut() {
                t.panel.set_show_hidden(on);
            }
        }
        self.load_all()
    }
```
In `act()`, replace `ToggleHidden` in the temporary arm:
```rust
            Action::ToggleHidden => {
                let on = !self.config.show_hidden;
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_show_hidden(h, on) {
                            log::warn!("config: {e}");
                        }
                    }
                    None => self.config.show_hidden = on,
                }
                return self.apply_hidden();
            }
```
(`set_show_hidden` is generated by the derive; it writes and updates the field.)

`CloseTab`:
```rust
            Action::CloseTab => {
                let i = self.panes[side].active_index();
                if !self.panes[side].close(i) {
                    if self.config.last_tab_close == LastTab::Home {
                        let home = self.config.home_dir.clone().unwrap_or_else(|| self.home.clone());
                        return self.load(side, home, None);
                    }
                    return Task::none();
                }
                return self.restore_scroll(side);
            }
```
The Ctrl+T path (`Action::NewTab`) is unchanged: `duplicate` clones the panel, so it inherits `show_hidden`.

`subscription()`:
```rust
        Subscription::batch([
            event::listen_with(route_event),
            self.core().watch_config::<Config>(APP_ID).map(|u| Message::Config(u.config)),
        ])
```
`next_id` starts at 0 and `next_id()` pre-increments, so tab ids start at 1 (the placeholder tabs with id 0 are replaced).

- [ ] **Step 5: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Manual check**

`cargo run -p shagoff-commander`:
- Open 2 tabs in different dirs, quit, restart: the tabs come back.
- `ls ~/.local/state/cosmic/io.github.shagovAlexei.cosmic-shagoff-commander/v1/` shows `panes` and `active`.
- `echo true > ~/.config/cosmic/io.github.shagovAlexei.cosmic-shagoff-commander/v1/show_hidden` while running: hidden files appear.

Don't send keys to the window. If you can't check something without the user, list it for TESTING.md.

- [ ] **Step 7: Commit**

```bash
cargo build -q && git add crates/app Cargo.lock && git commit -m "app: cosmic-config settings and state, remembered tabs, Ctrl+H, last-tab Ctrl+W option"
```

---

### Task 5: scan generations (tech debt) + rescan on tab switch

**Files:**
- Modify: `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `App::build`, `load_tab` (Task 4).
- Produces:
  - `Tab.pending: Option<u64>`;
  - `Message::Listed { side, tab, generation: u64, path, focus, result }`;
  - `App::tab_switched(&mut self, side: usize) -> Task<Message>`.

- [ ] **Step 1: Failing regression test**

```rust
fn entry(name: &str) -> Entry {
    Entry {
        name: name.into(),
        os_name: name.into(),
        ext: String::new(),
        size: 0,
        mtime: std::time::UNIX_EPOCH,
        kind: shagoff_core::listing::Kind::File,
        is_link: false,
        mode: 0o644,
    }
}

#[test]
fn regression_second_scan_of_same_path_wins() {
    let mut app = app_with(Config::default(), State::default());
    let (id, cwd) = (app.panes[0].active().id, app.panes[0].active().panel.cwd().to_path_buf());
    let _ = app.load(0, cwd.clone(), None);
    let first = app.panes[0].active().pending.unwrap();
    let _ = app.load(0, cwd.clone(), None);
    let second = app.panes[0].active().pending.unwrap();
    let listed = |generation, name: &str| Message::Listed {
        side: 0, tab: id, generation, path: cwd.clone(), focus: None, result: Ok(vec![entry(name)]),
    };
    let _ = app.update(listed(second, "new"));
    let _ = app.update(listed(first, "old")); // late, stale
    let names: Vec<_> = app.panes[0].active().panel.entries().iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"new".to_string()) && !names.contains(&"old".to_string()), "{names:?}");
}
```
(If `Entry` has more fields, fill them; if `Kind` lives elsewhere, adjust the path.)

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p shagoff-commander regression_second_scan`
Expected: compile error (no `generation` field); with path matching, both results would be accepted.

- [ ] **Step 3: Implement**

- `Tab.pending: Option<u64>` with doc `/// Generation of the scan in flight; any other result is stale.`
- `load_tab`: `let generation = self.next_id();` before borrowing the tab; set `t.pending = Some(generation)`; pass `generation` in `Message::Listed` (not `gen`: reserved in edition 2024).
- `Listed` arm: `if t.pending != Some(generation) { return Task::none(); }`.
- Tab switch:
```rust
    /// The newly shown tab was not watched while hidden: restore its scroll and rescan it.
    fn tab_switched(&mut self, side: usize) -> Task<Message> {
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        Task::batch([self.restore_scroll(side), self.load(side, cwd, None)])
    }
```
Use it in `SelectTab`, `CloseTabAt`, and in the `CloseTab` (successful close), `NextTab` and `PrevTab` arms in place of `restore_scroll(side)`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "app: scan generations instead of pending path; rescan tab on switch"
```

---

### Task 6: F3 / F4

**Files:**
- Modify: `crates/app/src/app.rs`, `crates/app/src/view.rs` (`fkey_bar`)

**Interfaces:**
- Consumes: `launch::command` (Task 2), `Action::View/Edit` (Task 3), `config.viewer/editor` (Task 4).
- Produces: `fn spawn_detached(argv: &[OsString]) -> std::io::Result<()>`, which replaces `open_detached`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn spawn_detached_reports_missing_program() {
    let argv = [OsString::from("/nonexistent/shagoff-test-prog"), "x".into()];
    assert!(spawn_detached(&argv).is_err());
}
```

- [ ] **Step 2: Run — expect FAIL** (`spawn_detached` not found)

Run: `cargo test -p shagoff-commander spawn_detached`

- [ ] **Step 3: Implement**

Replace `open_detached` with:
```rust
/// Run `argv` without blocking the UI or leaving a zombie.
fn spawn_detached(argv: &[OsString]) -> std::io::Result<()> {
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut child = std::process::Command::new(prog).args(args).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
```
Enter on a file:
```rust
                if let Some(file) = file
                    && let Err(err) = spawn_detached(&launch::command(&[], &["xdg-open"], &file))
```
New arms in `act()` (remove `View | Edit` from the temporary arm):
```rust
            Action::View | Action::Edit => {
                let file = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| panel.cwd().join(&e.os_name));
                if let Some(file) = file {
                    let argv = if action == Action::View {
                        launch::command(&self.config.viewer, &["xdg-open"], &file)
                    } else {
                        launch::command(&self.config.editor, &["cosmic-edit"], &file)
                    };
                    if let Err(err) = spawn_detached(&argv) {
                        t.error = Some(fl!("open-failed", err = err.to_string()));
                    }
                }
            }
```
(`panel`/`t` are borrowed from `self.panes`. If the borrow checker objects to reading `self.config` there, clone the two `Vec<String>` before taking `t`.)

`view.rs` `fkey_bar`: replace the two `disabled(...)` with `key(fl!("fkey-view"), Action::View)` and `key(fl!("fkey-edit"), Action::Edit)`, and delete the `disabled` closure.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app && git commit -m "app: F3 view and F4 edit via configured programs"
```

---

### Task 7: drive buttons, free space, Alt+F1/F2 dialog

**Files:**
- Modify: `crates/app/src/app.rs`, `crates/app/src/view.rs`, `crates/app/src/dialogs.rs`, both ftl files

**Interfaces:**
- Consumes: `drives::{Drive, parse, containing, space}`, `format::human` (Task 1); `Action::Drives(side)` (Task 3).
- Produces:
  - `App.drives: Vec<Drive>`, `App.space: [Option<(u64, u64)>; 2]`;
  - `Message::Drive(usize, usize)` (side, drive index);
  - `Dialog::Drives { side: usize, cursor: usize }`;
  - `App::go_drive(&mut self, side, i) -> Task<Message>`;
  - `App::refresh_drives(&mut self, side)`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn drive_dialog_navigates_and_opens() {
    let mut app = app_with(Config::default(), State::default());
    app.drives = vec![
        Drive { label: "/".into(), path: "/".into() },
        Drive { label: "tmp".into(), path: std::env::temp_dir() },
    ];
    let _ = app.update(Message::Key(Action::Drives(1)));
    assert!(matches!(app.dialog, Some(Dialog::Drives { side: 1, .. })));
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::Key(Action::Up)); // clamped at 0
    assert!(matches!(app.dialog, Some(Dialog::Drives { cursor: 0, .. })));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down)); // clamped at 1
    assert!(matches!(app.dialog, Some(Dialog::Drives { cursor: 1, .. })));
    let _ = app.update(Message::Key(Action::Enter));
    assert!(app.dialog.is_none());
    assert_eq!(app.active, 1);
    assert!(app.panes[1].active().pending.is_some());
}
```
Note: `build` calls `refresh_drives` for both sides, which reads the real `/proc/self/mounts`. The test overwrites `app.drives` after build, so that's harmless.

- [ ] **Step 2: Run — expect FAIL** (no `Drives` variant / field)

Run: `cargo test -p shagoff-commander drive_dialog`

- [ ] **Step 3: Implement**

ftl `en`:
```
disk-free = { $free } free of { $total }
drives = Drive
```
ftl `ru`:
```
disk-free = { $free } свободно из { $total }
drives = Диск
```

`dialogs.rs`: variant
```rust
    /// Alt+F1 / Alt+F2.
    Drives { side: usize, cursor: usize },
```
`view` gets `drives: &[Drive]` as a new parameter (update the call in `App::dialog`):
```rust
        Dialog::Drives { side, cursor } => {
            let mut list = column![].spacing(2);
            for (i, d) in drives.iter().enumerate() {
                let label = format!("{}   {}", d.label, d.path.display());
                let b = if i == *cursor { widget::button::suggested(label) } else { widget::button::text(label) };
                list = list.push(b.on_press(Message::Drive(*side, i)).width(cosmic::iced::Length::Fill));
            }
            widget::dialog().title(fl!("drives")).control(list).secondary_action(cancel).into()
        }
```

`app.rs`:
- Fields `drives: Vec<Drive>` (pub), `space: [Option<(u64, u64)>; 2]` (pub), initialised empty/None in `build`. At the end of `build`, before `load_all`: `app.refresh_drives(0); app.refresh_drives(1);`.
```rust
    /// Mounts change rarely and are cheap to read: re-read with every listing.
    fn refresh_drives(&mut self, side: usize) {
        if let Ok(m) = std::fs::read_to_string("/proc/self/mounts") {
            self.drives = drives::parse(&m, &self.home);
        }
        self.space[side] = drives::space(self.panes[side].active().panel.cwd());
    }

    fn go_drive(&mut self, side: usize, i: usize) -> Task<Message> {
        let Some(path) = self.drives.get(i).map(|d| d.path.clone()) else {
            return Task::none();
        };
        self.active = side;
        self.load(side, path, None)
    }
```
- `Listed` Ok branch: after `set_listing`, `let is_active = self.panes[side].active().id == tab;` (compute before the borrow ends, as `reveal` does). Then `if is_active { self.refresh_drives(side); }` before `return self.reveal(side, tab)`.
- `Message::Drive(side, i)`: `self.dialog = None; return self.go_drive(side, i);` — but only `if self.job.is_none()`.
- Key gate for the Drives dialog, at the top of the `if let Some(d) = &self.dialog` block (make it `&mut`):
```rust
                if let Some(Dialog::Drives { side, cursor }) = &mut self.dialog {
                    match action {
                        Action::Up => *cursor = cursor.saturating_sub(1),
                        Action::Down => *cursor = (*cursor + 1).min(self.drives.len().saturating_sub(1)),
                        Action::Enter => {
                            let (side, i) = (*side, *cursor);
                            self.dialog = None;
                            return self.go_drive(side, i);
                        }
                        _ => {}
                    }
                    return Task::none();
                }
```
- `act()`: `Action::Drives(s)` must open the dialog. Add to `dialog_for`: `Action::Drives(s) => Some(Dialog::Drives { side: s, cursor: drives::containing(&self.drives, self.panes[s].active().panel.cwd()).unwrap_or(0) })`. Remove `Drives(_)` from the temporary arm and add `Action::Drives(_) => {}` with the comment `// Opened by dialog_for above.` The cursor starts on the drive holding the pane's cwd.
- `submit_dialog`: `Dialog::Drives { side, cursor } => self.go_drive(side, cursor)`.

`view.rs` `pane()`: before the tab bar, `col = column![drive_bar(app, side)]`:
```rust
/// Drive buttons (the one holding cwd highlighted) and free space on the current disk.
fn drive_bar(app: &App, side: usize) -> Element<'_, Message> {
    let current = drives::containing(&app.drives, app.panes[side].active().panel.cwd());
    let mut bar = row![].spacing(2).align_y(Alignment::Center);
    for (i, d) in app.drives.iter().enumerate() {
        let b = if Some(i) == current {
            button::suggested(d.label.clone())
        } else {
            button::text(d.label.clone())
        };
        bar = bar.push(b.on_press(Message::Drive(side, i)));
    }
    bar = bar.push(widget::Space::new().width(Length::Fill));
    if let Some((free, total)) = app.space[side] {
        bar = bar.push(text(fl!("disk-free", free = format::human(free), total = format::human(total))).size(TEXT));
    }
    container(bar).padding([2, 6]).into()
}
```
// ponytail: many drives overflow the row; wrap in a horizontal scrollable when someone has more than fit.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Visual check**

Run the app and take a `grim` screenshot of its window region without focusing it or sending keys. Check that the drive row shows `/ ~ home SAVE_FLASH sys` with one highlighted and the free-space text on the right. If the window is hidden behind others, skip the screenshot and note it for TESTING.md.

- [ ] **Step 6: Commit**

```bash
git add crates/app && git commit -m "app: drive buttons with free space, Alt+F1/F2 drive list"
```

---

### Task 8: watcher

**Files:**
- Create: `crates/app/src/watcher.rs`
- Modify: `crates/app/src/main.rs` (`mod watcher;`), `crates/app/src/app.rs`, `crates/app/Cargo.toml` (notify, tokio time)

**Interfaces:**
- Consumes: `load` (Tasks 4–5), `App.job`.
- Produces:
  - `watcher::watch(side: usize, dir: PathBuf) -> Subscription<Message>`;
  - `watcher::relevant(kind: &notify::EventKind) -> bool`;
  - `Message::Changed(usize)`.

- [ ] **Step 1: Deps**

`crates/app/Cargo.toml`: `notify = "8.2"`; tokio features `["rt", "time"]`.

- [ ] **Step 2: Failing test** (in `watcher.rs`, with `relevant` stubbed as `todo!()`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};

    #[test]
    fn open_and_read_events_are_ignored() {
        // Our own scan opens the dir (IN_OPEN): reacting to it would rescan forever.
        assert!(!relevant(&EventKind::Access(AccessKind::Open(AccessMode::Any))));
        assert!(!relevant(&EventKind::Access(AccessKind::Close(AccessMode::Read))));
        assert!(!relevant(&EventKind::Access(AccessKind::Read)));
    }

    #[test]
    fn changes_are_relevant() {
        assert!(relevant(&EventKind::Create(CreateKind::File)));
        assert!(relevant(&EventKind::Remove(RemoveKind::Any)));
        assert!(relevant(&EventKind::Modify(ModifyKind::Any)));
        assert!(relevant(&EventKind::Access(AccessKind::Close(AccessMode::Write))));
    }
}
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo test -p shagoff-commander watcher`

- [ ] **Step 4: Implement `watcher.rs`**

```rust
//! Auto-refresh: a notify watch on one pane's cwd, debounced into `Message::Changed(side)`.

use crate::app::Message;
use cosmic::iced::Subscription;
use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Quiet time before a rescan, and the longest a stream of events can delay it.
const QUIET: Duration = Duration::from_millis(200);
const MAX_DELAY: Duration = Duration::from_secs(1);

/// Opening or reading is not a change; closing after a write is.
pub fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Recreated whenever `(side, dir)` changes (iced keys subscriptions by their data).
pub fn watch(side: usize, dir: PathBuf) -> Subscription<Message> {
    Subscription::run_with((side, dir), |(side, dir)| {
        let (side, dir) = (*side, dir.clone());
        cosmic::iced::stream::channel(1, async move |mut out| {
            let (tx, mut rx) = mpsc::unbounded();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok_and(|e| relevant(&e.kind)) {
                    let _ = tx.unbounded_send(());
                }
            });
            // Keep the watcher alive for the life of the stream.
            let _watcher = match watcher.and_then(|mut w| w.watch(&dir, RecursiveMode::NonRecursive).map(|()| w)) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("watch {}: {e}", dir.display());
                    return std::future::pending().await;
                }
            };
            while rx.next().await.is_some() {
                let start = Instant::now();
                while start.elapsed() < MAX_DELAY {
                    match tokio::time::timeout(QUIET, rx.next()).await {
                        Ok(Some(())) => continue,
                        _ => break,
                    }
                }
                if out.send(Message::Changed(side)).await.is_err() {
                    break;
                }
            }
        })
    })
}
```
If `run_with`'s `fn(&D) -> S` pointer rejects the closure (it captures nothing, so it should coerce), turn it into a named `fn build(data: &(usize, PathBuf)) -> impl Stream<Item = Message>`.

`app.rs`:
- `Message::Changed(usize)` → in `handle`: `Message::Changed(side) if self.job.is_none() => { let cwd = …active cwd…; return self.load(side, cwd, None); }` plus `Message::Changed(_) => {}`.
- `subscription()`:
```rust
        let mut subs = vec![
            event::listen_with(route_event),
            self.core().watch_config::<Config>(APP_ID).map(|u| Message::Config(u.config)),
        ];
        if self.job.is_none() {
            // Paused during file operations: finish_job rescans both panes anyway.
            for side in 0..2 {
                subs.push(watcher::watch(side, self.panes[side].active().panel.cwd().to_path_buf()));
            }
        }
        Subscription::batch(subs)
```

- [ ] **Step 5: Run — expect PASS**

Run: `cargo test -p shagoff-commander && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Manual check (no keys sent)**

Start the app with `cargo run -p shagoff-commander -- /tmp/shagoff-watch` (create the dir first). From a terminal:
- `touch /tmp/shagoff-watch/a`: `a` appears within ~1 s.
- `rm /tmp/shagoff-watch/a`: it disappears.

Confirm in a screenshot if the window is visible. Also check that the app is idle with nothing changing (`top -p $(pgrep shagoff-commander)` shows ~0% CPU), which proves there is no rescan loop.

- [ ] **Step 7: Commit**

```bash
cargo build -q && git add crates/app Cargo.lock && git commit -m "app: auto-refresh panes on directory changes"
```

---

### Task 9: docs

**Files:**
- Modify: `TESTING.md`, `.claude/docs/ROADMAP.md`, `.claude/docs/tc-reference.md`, `CLAUDE.md`

- [ ] **Step 1: TESTING.md** — add a section «Фаза 6» with these items:
  - Вкладки и активная панель восстанавливаются после перезапуска; удалённый каталог → ближайший родитель
  - Правка `~/.config/cosmic/io.github.shagovAlexei.cosmic-shagoff-commander/v1/show_hidden` на лету показывает/прячет скрытые
  - Ctrl+H — скрытые во всех вкладках обеих панелей, после перезапуска сохраняется
  - Ctrl+W на последней вкладке: по умолчанию ничего; `last_tab_close` = `Home` → домашний (или `home_dir`)
  - F3 — файл открывается программой по умолчанию; F4 — cosmic-edit; на каталоге ничего; несуществующая программа в `editor` → ошибка в строке состояния
  - Кнопки дисков: `/ ~ home SAVE_FLASH sys`, текущий подсвечен, свободное место справа; клик переходит в корень диска
  - Флешка: вставить/извлечь — кнопка появляется/пропадает после перехода в любой каталог
  - Alt+F1 / Alt+F2 — список дисков, ↑/↓/Enter/Escape (не перехватывает ли COSMIC)
  - Автообновление: `touch`/`rm` в открытом каталоге из терминала — видно за ~1 с; при копировании большого файла панель не дёргается; в простое CPU ~0 %

  Add the row `regression_second_scan_of_same_path_wins` (scan generations) to the regression table.
- [ ] **Step 2: ROADMAP.md**
  - Tick phase 6 and its sub-items.
  - Tick these tech-debt items:
    - Фаза 6 (watcher) generations;
    - Фаза 6 keymap Alt;
    - `Action::Cancel` removed.
- [ ] **Step 3: tc-reference.md**
  - Mark F3, F4, Alt+F1/F2 and Ctrl+H as done.
  - Add the result of the COSMIC interception check for Alt+F1/F2 if known.
- [ ] **Step 4: CLAUDE.md**
  - In "Implemented so far", add the core modules `drives`, `session`, `launch` and the app modules `config.rs`, `watcher.rs`.
  - Remove "No config yet".
  - Fix the stale `ops.rs` line: part files are `.<name>.<pid>.<n>.shagoff-part`.
- [ ] **Step 5: verify + commit**

```bash
just verify && git add -A && git commit -m "docs: phase 6 testing, roadmap, tc-reference, CLAUDE.md"
```
