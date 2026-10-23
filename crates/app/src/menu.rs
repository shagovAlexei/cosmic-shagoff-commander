//! Main menu in the header bar, like TC's. Every item sends the same `Action` as its key.

use crate::app::Message;
use crate::fl;
use crate::keymap::Action;
use cosmic::Element;
use cosmic::iced::keyboard::Key;
use cosmic::iced::keyboard::key::{Code, Named};
use cosmic::widget::menu::key_bind::Modifier;
use cosmic::widget::menu::{self, ItemHeight, ItemWidth, KeyBind};
use shagoff_core::sort::SortKey;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAct {
    Key(Action),
    /// Context menu "Open": the entry, never a command typed in the command line (Enter's job).
    Open,
    Exit,
}

impl menu::Action for MenuAct {
    type Message = Message;
    fn message(&self) -> Message {
        match *self {
            MenuAct::Key(a) => Message::Key(a),
            MenuAct::Open => Message::OpenEntry,
            MenuAct::Exit => Message::Quit,
        }
    }
}

/// How a key is shown and how `keymap` sees it: letters and the numpad go by physical key.
/// The `Code` is what `keymap` matches; only the test that checks the table reads it.
#[derive(Clone, Copy)]
#[cfg_attr(not(test), expect(dead_code))]
enum K {
    Named(Named),
    /// A named key shown as a symbol (libcosmic prints `Named` with `{:?}`: "ArrowDown").
    Shown(Named, &'static str),
    Letter(Code, &'static str),
    Num(Code, &'static str),
}

/// The keys the menu shows. Checked against `keymap::action` by a test.
fn table() -> Vec<(Vec<Modifier>, K, MenuAct)> {
    use Modifier::{Alt, Ctrl, Shift};
    let a = MenuAct::Key;
    vec![
        (vec![], K::Named(Named::Enter), MenuAct::Open),
        (vec![Ctrl], K::Letter(Code::KeyX, "x"), a(Action::ClipCut)),
        (vec![Ctrl], K::Letter(Code::KeyC, "c"), a(Action::ClipCopy)),
        (vec![Ctrl], K::Letter(Code::KeyV, "v"), a(Action::ClipPaste)),
        (vec![], K::Named(Named::F3), a(Action::View)),
        (vec![], K::Named(Named::F4), a(Action::Edit)),
        (vec![Alt], K::Named(Named::Enter), a(Action::Properties)),
        (
            vec![Alt, Shift],
            K::Named(Named::Enter),
            a(Action::CountDirs),
        ),
        (vec![], K::Named(Named::F5), a(Action::Copy)),
        (vec![Shift], K::Named(Named::F5), a(Action::CopySame)),
        (vec![Ctrl, Shift], K::Named(Named::F5), a(Action::Link)),
        (vec![], K::Named(Named::F6), a(Action::Move)),
        (vec![Shift], K::Named(Named::F6), a(Action::Rename)),
        (vec![], K::Named(Named::F7), a(Action::Mkdir)),
        (vec![Shift], K::Named(Named::F4), a(Action::NewFile)),
        (vec![], K::Named(Named::F8), a(Action::Delete)),
        (vec![Shift], K::Named(Named::F8), a(Action::DeletePermanent)),
        (vec![Alt], K::Named(Named::F5), a(Action::Pack)),
        (vec![Alt], K::Named(Named::F9), a(Action::Unpack)),
        (
            vec![Ctrl],
            K::Letter(Code::KeyM, "m"),
            a(Action::MultiRename),
        ),
        (vec![], K::Named(Named::F11), a(Action::CompareFiles)),
        (vec![], K::Named(Named::F9), a(Action::CopyNames)),
        (vec![], K::Named(Named::F10), a(Action::CopyPaths)),
        (vec![Alt], K::Named(Named::F4), MenuAct::Exit),
        (
            vec![],
            K::Num(Code::NumpadAdd, "Num +"),
            a(Action::SelectGroup),
        ),
        (
            vec![],
            K::Num(Code::NumpadSubtract, "Num −"),
            a(Action::UnselectGroup),
        ),
        (vec![Ctrl], K::Letter(Code::KeyA, "a"), a(Action::SelectAll)),
        (
            vec![Ctrl, Shift],
            K::Num(Code::NumpadAdd, "Num +"),
            a(Action::ThumbZoomIn),
        ),
        (
            vec![Ctrl, Shift],
            K::Num(Code::NumpadSubtract, "Num −"),
            a(Action::ThumbZoomOut),
        ),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyR, "r"),
            a(Action::ReloadThumbs),
        ),
        (
            vec![Ctrl],
            K::Num(Code::NumpadSubtract, "Num −"),
            a(Action::UnselectAll),
        ),
        (
            vec![],
            K::Num(Code::NumpadMultiply, "Num *"),
            a(Action::Invert),
        ),
        (
            vec![],
            K::Num(Code::NumpadDivide, "Num /"),
            a(Action::RestoreMarks),
        ),
        (
            vec![Alt],
            K::Num(Code::NumpadAdd, "Num +"),
            a(Action::MarkExt),
        ),
        (
            vec![Alt],
            K::Num(Code::NumpadSubtract, "Num −"),
            a(Action::UnmarkExt),
        ),
        (vec![Ctrl], K::Letter(Code::KeyL, "l"), a(Action::Occupied)),
        (
            vec![Alt, Shift],
            K::Named(Named::F9),
            a(Action::TestArchives),
        ),
        (
            vec![Alt, Shift],
            K::Named(Named::F5),
            a(Action::MoveToArchive),
        ),
        (vec![Alt], K::Named(Named::F10), a(Action::TreeDialog)),
        (vec![Ctrl], K::Named(Named::F8), a(Action::TreePanel)),
        (vec![Ctrl, Shift], K::Named(Named::F8), a(Action::SideTree)),
        (vec![Ctrl], K::Named(Named::F10), a(Action::ShowAll)),
        (vec![Ctrl], K::Named(Named::F11), a(Action::ShowPrograms)),
        (vec![Ctrl], K::Named(Named::F12), a(Action::ShowMask)),
        (
            vec![Ctrl],
            K::Shown(Named::ArrowLeft, "←"),
            a(Action::OpenLeft),
        ),
        (
            vec![Ctrl],
            K::Shown(Named::ArrowRight, "→"),
            a(Action::OpenRight),
        ),
        (vec![Shift], K::Named(Named::F2), a(Action::CompareLists)),
        (vec![Alt], K::Named(Named::F7), a(Action::FindFiles)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyS, "s"),
            a(Action::SyncDirs),
        ),
        (vec![Ctrl], K::Letter(Code::KeyD, "d"), a(Action::Hotlist)),
        (
            vec![Alt],
            K::Shown(Named::ArrowDown, "↓"),
            a(Action::HistoryList),
        ),
        (vec![Ctrl], K::Letter(Code::KeyU, "u"), a(Action::SwapPanes)),
        (vec![Ctrl], K::Letter(Code::KeyQ, "q"), a(Action::QuickView)),
        (vec![Ctrl], K::Letter(Code::KeyB, "b"), a(Action::Branch)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyB, "b"),
            a(Action::BranchSel),
        ),
        (
            vec![Ctrl],
            K::Letter(Code::KeyS, "s"),
            a(Action::QuickFilter),
        ),
        (vec![Ctrl], K::Letter(Code::KeyR, "r"), a(Action::Reload)),
        (vec![Ctrl], K::Letter(Code::KeyT, "t"), a(Action::NewTab)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyT, "t"),
            a(Action::NewTabBg),
        ),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyA, "a"),
            a(Action::TabsList),
        ),
        (vec![Ctrl], K::Letter(Code::KeyW, "w"), a(Action::CloseTab)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyW, "w"),
            a(Action::CloseOtherTabs),
        ),
        (
            vec![Ctrl, Shift],
            K::Shown(Named::PageUp, "PgUp"),
            a(Action::MoveTabLeft),
        ),
        (
            vec![Ctrl, Shift],
            K::Shown(Named::PageDown, "PgDn"),
            a(Action::MoveTabRight),
        ),
        (
            vec![Ctrl],
            K::Shown(Named::ArrowUp, "↑"),
            a(Action::TabOpen),
        ),
        (
            vec![Ctrl, Shift],
            K::Shown(Named::ArrowUp, "↑"),
            a(Action::TabOpenOther),
        ),
        (vec![Ctrl], K::Letter(Code::KeyF, "f"), a(Action::Connect)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyF, "f"),
            a(Action::Disconnect),
        ),
        (vec![Alt], K::Named(Named::F1), a(Action::Drives(0))),
        (vec![Alt], K::Named(Named::F2), a(Action::Drives(1))),
        (
            vec![Ctrl],
            K::Letter(Code::KeyH, "h"),
            a(Action::ToggleHidden),
        ),
        (vec![Ctrl], K::Letter(Code::KeyO, "o"), a(Action::TermFull)),
        (
            vec![Ctrl],
            K::Letter(Code::Backquote, "`"),
            a(Action::ToggleTerminal),
        ),
        (vec![], K::Named(Named::F1), a(Action::Help)),
        (vec![Ctrl], K::Named(Named::F1), a(Action::ViewBrief)),
        (
            vec![Ctrl, Shift],
            K::Named(Named::F1),
            a(Action::ViewThumbs),
        ),
        (vec![Ctrl], K::Named(Named::F2), a(Action::ViewFull)),
        (vec![Ctrl], K::Letter(Code::Comma, ","), a(Action::Settings)),
        (
            vec![Ctrl],
            K::Named(Named::F3),
            a(Action::Sort(SortKey::Name)),
        ),
        (
            vec![Ctrl],
            K::Named(Named::F4),
            a(Action::Sort(SortKey::Ext)),
        ),
        (
            vec![Ctrl],
            K::Named(Named::F5),
            a(Action::Sort(SortKey::Date)),
        ),
        (
            vec![Ctrl],
            K::Named(Named::F7),
            a(Action::Sort(SortKey::Unsorted)),
        ),
        (
            vec![Ctrl],
            K::Named(Named::F6),
            a(Action::Sort(SortKey::Size)),
        ),
    ]
}

fn key_binds() -> HashMap<KeyBind, MenuAct> {
    let mut binds: HashMap<KeyBind, MenuAct> = table()
        .into_iter()
        .map(|(modifiers, k, act)| {
            let key = match k {
                K::Named(n) => Key::Named(n),
                K::Letter(_, s) | K::Num(_, s) | K::Shown(_, s) => Key::Character(s.into()),
            };
            (KeyBind { modifiers, key }, act)
        })
        .collect();
    // The user's own keys (Configuration → Hotkeys) shown in place of the defaults they replace.
    for (b, act) in crate::keymap::own() {
        let kb = shown(&b);
        binds.remove(&kb);
        if let Some(a) = act {
            binds.retain(|_, v| *v != MenuAct::Key(a));
            binds.insert(kb, MenuAct::Key(a));
        }
    }
    binds
}

/// A `Bind` as the menu shows a key (as `table` writes them).
fn shown(b: &shagoff_core::hotkey::Bind) -> KeyBind {
    let mut modifiers = Vec::new();
    for (on, m) in [
        (b.ctrl, Modifier::Ctrl),
        (b.alt, Modifier::Alt),
        (b.shift, Modifier::Shift),
    ] {
        if on {
            modifiers.push(m);
        }
    }
    let named = |n| Key::Named(n);
    let ch = |s: &str| Key::Character(s.into());
    let key = match b.key.as_str() {
        k if k.chars().count() == 1 => ch(&k.to_lowercase()),
        "Up" => ch("↑"),
        "Down" => ch("↓"),
        "Left" => ch("←"),
        "Right" => ch("→"),
        "PageUp" => ch("PgUp"),
        "PageDown" => ch("PgDn"),
        "NumpadAdd" => ch("Num +"),
        "NumpadSubtract" => ch("Num −"),
        "NumpadMultiply" => ch("Num *"),
        "Backquote" => ch("`"),
        "Comma" => ch(","),
        "Enter" => named(Named::Enter),
        "Tab" => named(Named::Tab),
        "Space" => ch("Space"),
        "Backspace" => named(Named::Backspace),
        "Delete" => named(Named::Delete),
        "Insert" => named(Named::Insert),
        "Home" => named(Named::Home),
        "End" => named(Named::End),
        f if f.starts_with('F') && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => {
            let n = f[1..].parse::<u8>().unwrap_or(1);
            named(
                [
                    Named::F1,
                    Named::F2,
                    Named::F3,
                    Named::F4,
                    Named::F5,
                    Named::F6,
                    Named::F7,
                    Named::F8,
                    Named::F9,
                    Named::F10,
                    Named::F11,
                    Named::F12,
                ][usize::from(n - 1)],
            )
        }
        k => ch(k),
    };
    KeyBind { modifiers, key }
}

/// Every command the menus give, as ("menu → item", action): what a key can be set to (hotkeys).
pub fn commands() -> Vec<(String, Action)> {
    fn walk(path: &str, items: &[Item], out: &mut Vec<(String, Action)>) {
        for i in items {
            match i {
                menu::Item::Button(l, _, MenuAct::Key(a))
                | menu::Item::CheckBox(l, _, _, MenuAct::Key(a)) => {
                    if !out.iter().any(|(_, b)| b == a) {
                        out.push((format!("{path} → {l}"), *a));
                    }
                }
                menu::Item::Folder(l, sub) => walk(&format!("{path} → {l}"), sub, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for (title, items) in menus(Checks::default(), Lock::default()) {
        walk(&title, &items, &mut out);
    }
    out
}

type Item = menu::Item<MenuAct, String>;

/// The menu checkboxes' states (and the Start menu's items).
#[derive(Clone, Default)]
pub struct Checks {
    pub hidden: bool,
    pub divider_line: bool,
    /// View → Window parts: shown or not, in `Part::ALL` order.
    pub parts: [bool; crate::config::Part::ALL.len()],
    pub wide: bool,
    pub vertical: bool,
    pub side_tree: bool,
    /// View → Columns: shown or not, in `Col::ALL` order.
    pub cols: [bool; shagoff_core::columns::Col::ALL.len()],
    /// The view modes' names, and whether the columns shown are that mode's.
    pub modes: Vec<(String, bool)>,
    pub human_sizes: bool,
    pub active_frame: bool,
    /// The classic look: only there is «Frame around the active panel» offered (the modern
    /// look's cards have one of their own).
    pub classic: bool,
    pub terminal: bool,
    pub sync_dirs: bool,
    /// View → «Only selected files» in force in the active panel.
    pub only_marked: bool,
    /// View → switches: the ignore list, watching dirs, dirs by name, colors by type.
    pub ignore: bool,
    pub watch: bool,
    pub dirs_by_name: bool,
    pub type_colors: bool,
    /// The Start menu's item names (`None`: a line).
    pub start: Vec<Option<String>>,
    /// Ctrl+F10 / F11 / F12 in force in the active panel (0 / 1 / 2): its item checked.
    pub show: u8,
}

/// (title, items) of every menu.
fn menus(checks: Checks, lock: Lock) -> Vec<(String, Vec<Item>)> {
    let b = |label: String, a: Action| menu::Item::Button(label, None, MenuAct::Key(a));
    let sort = |label: String, k: SortKey| b(label, Action::Sort(k));
    vec![
        (
            fl!("menu-files"),
            vec![
                b(fl!("menu-view"), Action::View),
                b(fl!("menu-edit"), Action::Edit),
                b(fl!("menu-copy"), Action::Copy),
                b(fl!("menu-copy-same"), Action::CopySame),
                b(fl!("menu-copy-left"), Action::TransferLeft),
                b(fl!("menu-copy-right"), Action::TransferRight),
                b(fl!("menu-link"), Action::Link),
                b(fl!("menu-move"), Action::Move),
                b(fl!("menu-rename"), Action::Rename),
                b(fl!("menu-properties"), Action::Properties),
                b(fl!("menu-set-date"), Action::SetDate),
                b(fl!("menu-count-dirs"), Action::CountDirs),
                b(fl!("menu-occupied"), Action::Occupied),
                b(fl!("menu-mkdir"), Action::Mkdir),
                b(fl!("menu-new-file"), Action::NewFile),
                b(fl!("menu-new-from-template"), Action::NewFromTemplate),
                b(fl!("menu-delete"), Action::Delete),
                b(fl!("menu-delete-permanent"), Action::DeletePermanent),
                menu::Item::Divider,
                b(fl!("menu-pack"), Action::Pack),
                b(fl!("menu-move-to-archive"), Action::MoveToArchive),
                b(fl!("menu-unpack"), Action::Unpack),
                b(fl!("menu-test-archives"), Action::TestArchives),
                b(fl!("menu-archive-encoding"), Action::ArchiveEncoding),
                b(fl!("menu-checksum-create"), Action::ChecksumCreate),
                b(fl!("menu-checksum-verify"), Action::ChecksumVerify),
                b(fl!("menu-split"), Action::SplitFile),
                b(fl!("menu-combine"), Action::CombineFiles),
                menu::Item::Divider,
                b(fl!("menu-multi-rename"), Action::MultiRename),
                b(fl!("menu-compare-files"), Action::CompareFiles),
                menu::Item::Divider,
                menu::Item::Button(fl!("menu-exit"), None, MenuAct::Exit),
            ],
        ),
        (
            fl!("menu-mark"),
            vec![
                b(fl!("menu-select-group"), Action::SelectGroup),
                b(fl!("menu-unselect-group"), Action::UnselectGroup),
                b(fl!("menu-select-all"), Action::SelectAll),
                b(fl!("menu-unselect-all"), Action::UnselectAll),
                b(fl!("menu-invert"), Action::Invert),
                b(fl!("menu-restore-marks"), Action::RestoreMarks),
                // Submenus: all of it in one column runs off a small screen.
                menu::Item::Folder(
                    fl!("menu-mark-alike"),
                    vec![
                        b(fl!("menu-mark-ext"), Action::MarkExt),
                        b(fl!("menu-unmark-ext"), Action::UnmarkExt),
                        b(fl!("menu-mark-name"), Action::MarkName),
                        b(fl!("menu-unmark-name"), Action::UnmarkName),
                        b(fl!("menu-mark-name-ext"), Action::MarkNameExt),
                        b(fl!("menu-unmark-name-ext"), Action::UnmarkNameExt),
                    ],
                ),
                menu::Item::Folder(
                    fl!("menu-marks-keep"),
                    vec![
                        b(fl!("menu-save-marks"), Action::SaveMarks),
                        b(fl!("menu-save-marks-file"), Action::SaveMarksToFile),
                        b(fl!("menu-load-marks-file"), Action::LoadMarksFromFile),
                        b(fl!("menu-load-marks-clip"), Action::LoadMarksFromClip),
                    ],
                ),
                b(fl!("menu-next-marked"), Action::NextMarked),
                b(fl!("menu-prev-marked"), Action::PrevMarked),
                menu::Item::Divider,
                menu::Item::Folder(
                    fl!("menu-compare-lists"),
                    vec![
                        b(fl!("menu-compare-files"), Action::CompareLists),
                        b(fl!("menu-compare-subdirs"), Action::CompareListsSubdirs),
                        b(fl!("menu-compare-diff-only"), Action::CompareListsDiffOnly),
                    ],
                ),
                menu::Item::Divider,
                menu::Item::Folder(
                    fl!("menu-copy-to-clip"),
                    vec![
                        b(fl!("menu-copy-names"), Action::CopyNames),
                        b(fl!("menu-copy-paths"), Action::CopyPaths),
                        b(fl!("menu-copy-src-path"), Action::CopySrcPath),
                        b(fl!("menu-copy-trg-path"), Action::CopyTrgPath),
                        b(fl!("menu-copy-dirs"), Action::CopyDirsOfFiles),
                        b(fl!("menu-copy-details"), Action::CopyDetails),
                        b(fl!("menu-copy-details-hdr"), Action::CopyDetailsHdr),
                        menu::Item::Divider,
                        b(fl!("menu-save-details"), Action::SaveDetails),
                        b(fl!("menu-save-details-hdr"), Action::SaveDetailsHdr),
                    ],
                ),
                menu::Item::Divider,
                b(fl!("menu-clip-cut"), Action::ClipCut),
                b(fl!("menu-clip-copy"), Action::ClipCopy),
                b(fl!("menu-clip-paste"), Action::ClipPaste),
            ],
        ),
        (
            fl!("menu-commands"),
            vec![
                b(fl!("menu-find"), Action::FindFiles),
                b(fl!("menu-sync"), Action::SyncDirs),
                menu::Item::Divider,
                b(fl!("menu-hotlist"), Action::Hotlist),
                b(fl!("menu-tree"), Action::TreeDialog),
                b(fl!("menu-history"), Action::HistoryList),
                b(fl!("menu-swap"), Action::SwapPanes),
                menu::Item::Folder(
                    fl!("menu-focus"),
                    vec![
                        b(fl!("menu-focus-left"), Action::FocusLeft),
                        b(fl!("menu-focus-right"), Action::FocusRight),
                        b(fl!("menu-focus-cmdline"), Action::FocusCmdLine),
                    ],
                ),
                b(fl!("menu-match-src"), Action::MatchSrc),
                menu::Item::CheckBox(
                    fl!("menu-sync-change-dir"),
                    None,
                    checks.sync_dirs,
                    MenuAct::Key(Action::SyncChangeDir),
                ),
                b(fl!("menu-open-left"), Action::OpenLeft),
                b(fl!("menu-open-right"), Action::OpenRight),
                b(fl!("menu-edit-path"), Action::EditPath),
                b(fl!("menu-open-trash"), Action::OpenTrash),
                menu::Item::Divider,
                b(fl!("menu-filter"), Action::QuickFilter),
                b(fl!("menu-reload"), Action::Reload),
                menu::Item::Divider,
                b(fl!("menu-open-terminal"), Action::OpenTerminal),
                b(fl!("menu-pick-terminal"), Action::PickTerminal),
                b(fl!("menu-pick-shell"), Action::PickShell),
            ],
        ),
        (
            fl!("menu-tabs"),
            vec![
                b(fl!("menu-tabs-list"), Action::TabsList),
                b(fl!("menu-new-tab"), Action::NewTab),
                b(fl!("menu-new-tab-bg"), Action::NewTabBg),
                b(fl!("menu-tab-open"), Action::TabOpen),
                menu::Item::Folder(
                    fl!("menu-tabs-other"),
                    vec![
                        b(fl!("menu-tab-open-other"), Action::TabOpenOther),
                        b(fl!("menu-new-tab-other"), Action::NewTabOther),
                        b(fl!("menu-tab-copy-other"), Action::TabCopyOther),
                        b(fl!("menu-tab-move-other"), Action::TabMoveOther),
                    ],
                ),
                menu::Item::Divider,
                menu::Item::CheckBox(
                    fl!("menu-tab-lock"),
                    None,
                    lock.locked,
                    MenuAct::Key(Action::TabLock),
                ),
                menu::Item::CheckBox(
                    fl!("menu-tab-roam"),
                    None,
                    lock.roam,
                    MenuAct::Key(Action::TabRoam),
                ),
                b(fl!("menu-go-locked-dir"), Action::GoToLockedDir),
                b(fl!("menu-tab-rename"), Action::TabRename),
                b(fl!("menu-tab-left"), Action::MoveTabLeft),
                b(fl!("menu-tab-right"), Action::MoveTabRight),
                menu::Item::Divider,
                b(fl!("menu-close-tab"), Action::CloseTab),
                b(fl!("menu-close-other-tabs"), Action::CloseOtherTabs),
                menu::Item::Folder(
                    fl!("menu-all-tabs"),
                    vec![
                        b(fl!("menu-lock-all-tabs"), Action::LockAllTabs),
                        b(fl!("menu-roam-all-tabs"), Action::RoamAllTabs),
                        b(fl!("menu-unlock-all-tabs"), Action::UnlockAllTabs),
                        menu::Item::Divider,
                        b(fl!("menu-close-duplicate-tabs"), Action::CloseDuplicateTabs),
                        b(fl!("menu-close-all-tabs"), Action::CloseAllTabs),
                    ],
                ),
            ],
        ),
        (
            fl!("menu-net"),
            vec![
                b(fl!("menu-connect"), Action::Connect),
                b(fl!("menu-disconnect"), Action::Disconnect),
                menu::Item::Divider,
                b(fl!("menu-drive-left"), Action::Drives(0)),
                b(fl!("menu-drive-right"), Action::Drives(1)),
            ],
        ),
        (
            fl!("menu-show"),
            vec![
                menu::Item::CheckBox(
                    fl!("menu-hidden"),
                    None,
                    checks.hidden,
                    MenuAct::Key(Action::ToggleHidden),
                ),
                menu::Item::CheckBox(
                    fl!("settings-divider-line"),
                    None,
                    checks.divider_line,
                    MenuAct::Key(Action::ToggleDividerLine),
                ),
                menu::Item::CheckBox(
                    fl!("settings-human-sizes"),
                    None,
                    checks.human_sizes,
                    MenuAct::Key(Action::ToggleHumanSizes),
                ),
                menu::Item::CheckBox(
                    fl!("menu-builtin-terminal"),
                    None,
                    checks.terminal,
                    MenuAct::Key(Action::ToggleTerminal),
                ),
            ]
            .into_iter()
            .chain(checks.classic.then(|| {
                menu::Item::CheckBox(
                    fl!("settings-active-frame"),
                    None,
                    checks.active_frame,
                    MenuAct::Key(Action::ToggleActiveFrame),
                )
            }))
            .chain([
                menu::Item::Folder(
                    fl!("menu-window-parts"),
                    (crate::config::Part::ALL.into_iter().zip(checks.parts))
                        .filter(|(p, _)| checks.classic || *p != crate::config::Part::Toolbar)
                        .map(|(p, on)| {
                            menu::Item::CheckBox(p.label(), None, on, MenuAct::Key(Action::Vis(p)))
                        })
                        .collect(),
                ),
                menu::Item::Folder(
                    fl!("menu-panels"),
                    vec![
                        menu::Item::CheckBox(
                            fl!("menu-side-tree"),
                            None,
                            checks.side_tree,
                            MenuAct::Key(Action::SideTree),
                        ),
                        b(fl!("menu-split-50"), Action::Split50),
                        menu::Item::CheckBox(
                            fl!("menu-vertical"),
                            None,
                            checks.vertical,
                            MenuAct::Key(Action::VerticalPanels),
                        ),
                        menu::Item::CheckBox(
                            fl!("menu-wide"),
                            None,
                            checks.wide,
                            MenuAct::Key(Action::Wide),
                        ),
                    ],
                ),
                menu::Item::Divider,
                b(fl!("menu-term-full"), Action::TermFull),
                b(fl!("menu-quick-view"), Action::QuickView),
                b(fl!("menu-branch"), Action::Branch),
                b(fl!("menu-branch-sel"), Action::BranchSel),
                b(fl!("menu-brief"), Action::ViewBrief),
                b(fl!("menu-full"), Action::ViewFull),
                menu::Item::Folder(
                    fl!("menu-thumbs-sub"),
                    vec![
                        b(fl!("menu-thumbs"), Action::ViewThumbs),
                        b(fl!("menu-thumb-zoom-in"), Action::ThumbZoomIn),
                        b(fl!("menu-thumb-zoom-out"), Action::ThumbZoomOut),
                        b(fl!("menu-thumb-reload"), Action::ReloadThumbs),
                    ],
                ),
                b(fl!("menu-tree-panel"), Action::TreePanel),
                menu::Item::Folder(
                    fl!("menu-columns"),
                    (shagoff_core::columns::Col::ALL.into_iter().zip(checks.cols))
                        .map(|(c, on)| {
                            menu::Item::CheckBox(
                                crate::view::col_label(c),
                                None,
                                on,
                                MenuAct::Key(Action::ToggleCol(c)),
                            )
                        })
                        .chain((!checks.modes.is_empty()).then_some(menu::Item::Divider))
                        .chain(checks.modes.iter().enumerate().map(|(i, (name, on))| {
                            menu::Item::CheckBox(
                                name.clone(),
                                None,
                                *on,
                                MenuAct::Key(Action::ViewMode(i)),
                            )
                        }))
                        .chain([
                            menu::Item::Divider,
                            b(fl!("menu-save-view-mode"), Action::SaveViewMode),
                        ])
                        .collect(),
                ),
                b(fl!("menu-reset-columns"), Action::ResetColumns),
                menu::Item::Divider,
                menu::Item::CheckBox(
                    fl!("menu-show-all"),
                    None,
                    checks.show == 0,
                    MenuAct::Key(Action::ShowAll),
                ),
                menu::Item::CheckBox(
                    fl!("menu-show-programs"),
                    None,
                    checks.show == 1,
                    MenuAct::Key(Action::ShowPrograms),
                ),
                menu::Item::CheckBox(
                    fl!("menu-show-mask"),
                    None,
                    checks.show == 2,
                    MenuAct::Key(Action::ShowMask),
                ),
                menu::Item::CheckBox(
                    fl!("menu-show-only-marked"),
                    None,
                    checks.only_marked,
                    MenuAct::Key(Action::ShowOnlyMarked),
                ),
                menu::Item::Divider,
                sort(fl!("menu-sort-name"), SortKey::Name),
                sort(fl!("menu-sort-ext"), SortKey::Ext),
                sort(fl!("menu-sort-date"), SortKey::Date),
                sort(fl!("menu-sort-size"), SortKey::Size),
                sort(fl!("menu-sort-unsorted"), SortKey::Unsorted),
                menu::Item::Folder(
                    fl!("menu-switches"),
                    vec![
                        menu::Item::CheckBox(
                            fl!("settings-ignore-on"),
                            None,
                            checks.ignore,
                            MenuAct::Key(Action::SwitchIgnoreList),
                        ),
                        menu::Item::CheckBox(
                            fl!("settings-watch-dirs"),
                            None,
                            checks.watch,
                            MenuAct::Key(Action::SwitchWatchDirs),
                        ),
                        menu::Item::CheckBox(
                            fl!("settings-dirs-by-name"),
                            None,
                            checks.dirs_by_name,
                            MenuAct::Key(Action::SwitchDirSort),
                        ),
                        menu::Item::CheckBox(
                            fl!("settings-type-colors"),
                            None,
                            checks.type_colors,
                            MenuAct::Key(Action::SwitchTypeColors),
                        ),
                    ],
                ),
            ])
            .collect(),
        ),
        (
            fl!("menu-config"),
            vec![
                b(fl!("menu-settings"), Action::Settings),
                b(fl!("menu-toolbar"), Action::ConfigureToolbar),
                b(fl!("menu-colors"), Action::ConfigureColors),
                b(fl!("menu-keys"), Action::ConfigureKeys),
            ],
        ),
        (
            fl!("menu-start"),
            [b(fl!("menu-start-edit"), Action::ConfigureStartMenu)]
                .into_iter()
                .chain((!checks.start.is_empty()).then_some(menu::Item::Divider))
                .chain(checks.start.iter().enumerate().map(|(i, l)| match l {
                    None => menu::Item::Divider,
                    Some(l) => b(l.clone(), Action::UserMenu(i)),
                }))
                .collect(),
        ),
        (
            fl!("menu-help-root"),
            vec![
                b(fl!("menu-help"), Action::Help),
                menu::Item::Divider,
                b(fl!("donate-title"), Action::Donate),
                b(fl!("menu-about"), Action::About),
            ],
        ),
    ]
}

/// What was right-clicked: the context menu's items depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    /// A file or dir (or the marked ones); `archive`: opens as a folder (offer unpacking).
    Entry {
        dir: bool,
        archive: bool,
    },
    /// An item right in the trash.
    Trash,
    /// `..` or the empty space below the rows: the dir itself.
    Dir,
    Tab {
        lock: Lock,
    },
}

/// A tab's lock as its menus show it: locked (TC), or locked with dir changes allowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lock {
    pub locked: bool,
    pub roam: bool,
}

fn context_items(ctx: Ctx) -> Vec<Item> {
    let b = |label: String, a: Action| menu::Item::Button(label, None, MenuAct::Key(a));
    match ctx {
        Ctx::Entry { dir, archive } => {
            let mut v = vec![
                menu::Item::Button(fl!("menu-open"), None, MenuAct::Open),
                b(fl!("menu-view"), Action::View),
            ];
            // An archive is opened as a folder, not edited; 12 items fit a 530 px window.
            if !dir && !archive {
                v.push(b(fl!("menu-edit"), Action::Edit));
            }
            if !dir {
                v.push(b(fl!("menu-open-with"), Action::OpenWith));
            }
            v.extend([
                menu::Item::Divider,
                b(fl!("menu-copy"), Action::Copy),
                b(fl!("menu-move"), Action::Move),
                b(fl!("menu-rename"), Action::Rename),
                b(fl!("menu-delete"), Action::Delete),
                menu::Item::Divider,
                b(fl!("menu-clip-cut"), Action::ClipCut),
                b(fl!("menu-clip-copy"), Action::ClipCopy),
                b(fl!("menu-clip-paste"), Action::ClipPaste),
                menu::Item::Divider,
            ]);
            v.push(if archive {
                b(fl!("menu-unpack"), Action::Unpack)
            } else {
                b(fl!("menu-pack"), Action::Pack)
            });
            // Short enough for a 530 px window: Shift+F5, F9 / F10 stay in the main menu.
            v.push(menu::Item::Divider);
            if dir || archive {
                v.push(b(fl!("menu-tab-open"), Action::TabOpen));
            }
            v.push(b(fl!("menu-properties"), Action::Properties));
            v
        }
        Ctx::Trash => vec![
            b(fl!("menu-restore"), Action::Restore),
            b(fl!("menu-view"), Action::View),
            menu::Item::Divider,
            b(fl!("menu-copy"), Action::Copy),
            b(fl!("menu-move"), Action::Move),
            b(fl!("menu-delete"), Action::Delete),
            menu::Item::Divider,
            b(fl!("menu-empty-trash"), Action::EmptyTrash),
            b(fl!("menu-properties"), Action::Properties),
        ],
        Ctx::Dir => vec![
            b(fl!("menu-mkdir"), Action::Mkdir),
            b(fl!("menu-clip-paste"), Action::ClipPaste),
            menu::Item::Divider,
            b(fl!("menu-reload"), Action::Reload),
            b(fl!("menu-select-all"), Action::SelectAll),
            b(fl!("menu-count-dirs"), Action::CountDirs),
            b(fl!("menu-find"), Action::FindFiles),
            b(fl!("menu-new-tab"), Action::NewTab),
        ],
        Ctx::Tab { lock } => vec![
            b(fl!("menu-new-tab"), Action::NewTab),
            b(fl!("menu-reload"), Action::Reload),
            menu::Item::Divider,
            // Short: libcosmic's context menu is 240 px wide, and these are about the tab already.
            b(fl!("ctx-tab-copy-other"), Action::TabCopyOther),
            b(fl!("ctx-tab-move-other"), Action::TabMoveOther),
            menu::Item::Divider,
            menu::Item::CheckBox(
                fl!("menu-tab-lock"),
                None,
                lock.locked,
                MenuAct::Key(Action::TabLock),
            ),
            menu::Item::CheckBox(
                fl!("menu-tab-roam"),
                None,
                lock.roam,
                MenuAct::Key(Action::TabRoam),
            ),
            b(fl!("menu-tab-rename"), Action::TabRename),
            menu::Item::Divider,
            b(fl!("menu-close-tab"), Action::CloseTab),
            b(fl!("ctx-close-other-tabs"), Action::CloseOtherTabs),
        ],
    }
}

impl Ctx {
    /// The menu for the row `e` (not `..`) of the dir `cwd`.
    pub fn entry(e: &shagoff_core::listing::Entry, cwd: &std::path::Path) -> Self {
        if shagoff_core::trashcan::is_top(cwd) {
            return Ctx::Trash;
        }
        Ctx::Entry {
            dir: e.is_dir(),
            archive: shagoff_core::archive::Format::detect(&e.name).is_some_and(|f| f.is_tree()),
        }
    }
}

/// Shift+F10 / the Menu key: `ctx`'s menu as rows of a list dialog — (label, what it does), `None`
/// for a divider. libcosmic opens its context menu only on a right-button release.
pub fn context_rows(ctx: Ctx) -> Vec<(String, Option<MenuAct>)> {
    context_items(ctx)
        .into_iter()
        .filter_map(|it| match it {
            menu::Item::Button(label, _, act) | menu::Item::CheckBox(label, _, _, act) => {
                Some((label, Some(act)))
            }
            menu::Item::Divider => Some((String::new(), None)),
            _ => None,
        })
        .collect()
}

/// Right-click menu for `ctx`, keys shown from the same table as the main menu.
pub fn context(ctx: Ctx) -> Vec<menu::Tree<Message>> {
    menu::items(&key_binds(), context_items(ctx))
}

pub fn bar(checks: Checks, lock: Lock) -> Element<'static, Message> {
    let binds = key_binds();
    let roots = menus(checks, lock)
        .into_iter()
        .map(|(title, items)| {
            menu::Tree::with_children(Element::from(menu::root(title)), menu::items(&binds, items))
        })
        .collect();
    menu::bar(roots)
        .item_height(ItemHeight::Dynamic(40))
        .item_width(ItemWidth::Uniform(380))
        .spacing(4.0)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap;
    use cosmic::iced::keyboard::Modifiers;
    use cosmic::iced::keyboard::key::Physical;

    fn actions(ctx: Ctx) -> Vec<Action> {
        context_items(ctx)
            .into_iter()
            .filter_map(|i| match i {
                menu::Item::Button(_, _, MenuAct::Key(a))
                | menu::Item::CheckBox(_, _, _, MenuAct::Key(a)) => Some(a),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn context_menus_fit_what_was_clicked() {
        let file = actions(Ctx::Entry {
            dir: false,
            archive: false,
        });
        assert!(file.contains(&Action::Edit) && !file.contains(&Action::TabOpen));
        assert!(file.contains(&Action::OpenWith));
        assert!(!file.contains(&Action::Unpack));
        let dir = actions(Ctx::Entry {
            dir: true,
            archive: false,
        });
        assert!(!dir.contains(&Action::Edit) && dir.contains(&Action::TabOpen));
        assert!(!dir.contains(&Action::OpenWith));
        let zip = actions(Ctx::Entry {
            dir: false,
            archive: true,
        });
        assert!(zip.contains(&Action::Unpack) && zip.contains(&Action::TabOpen));
        assert!(actions(Ctx::Dir).contains(&Action::Mkdir));
        let tab = actions(Ctx::Tab {
            lock: Lock {
                locked: true,
                roam: false,
            },
        });
        for a in [
            Action::NewTab,
            Action::CloseTab,
            Action::TabLock,
            Action::TabRename,
        ] {
            assert!(tab.contains(&a), "{a:?}");
        }
        // a 530 px window holds about 12 items
        assert!(
            file.len() <= 12 && dir.len() <= 12 && zip.len() <= 12,
            "{}",
            zip.len()
        );
    }

    #[test]
    fn menu_keys_do_what_the_menu_says() {
        for (mods, k, act) in table() {
            let mut m = Modifiers::empty();
            for x in &mods {
                m |= match x {
                    Modifier::Ctrl => Modifiers::CTRL,
                    Modifier::Shift => Modifiers::SHIFT,
                    Modifier::Alt => Modifiers::ALT,
                    Modifier::Super => Modifiers::LOGO,
                };
            }
            let (key, physical) = match k {
                K::Named(n) | K::Shown(n, _) => (Key::Named(n), Physical::Code(Code::F35)),
                K::Letter(c, s) => (Key::Character(s.into()), Physical::Code(c)),
                K::Num(c, _) => (Key::Character("+".into()), Physical::Code(c)),
            };
            let got = keymap::action(&key, physical, m);
            match act {
                // Alt+F4 belongs to the compositor; the menu item quits directly.
                MenuAct::Exit => assert_eq!(got, None),
                MenuAct::Key(a) => assert_eq!(got, Some(a), "{mods:?} {key:?}"),
                // The key is Enter; the item opens the entry even with a typed command.
                MenuAct::Open => assert_eq!(got, Some(Action::Enter)),
            }
        }
    }

    #[test]
    fn keys_and_actions_are_unique() {
        // A HashMap: a repeated key would hide an item's label, a repeated action shows a random key.
        let t = table();
        assert_eq!(key_binds().len(), t.len());
        for (i, (_, _, a)) in t.iter().enumerate() {
            assert!(!t[..i].iter().any(|(_, _, b)| b == a), "{a:?} twice");
        }
    }

    #[test]
    fn arrow_shown_as_arrow() {
        let shown: Vec<String> = key_binds()
            .into_iter()
            .filter(|(_, a)| *a == MenuAct::Key(Action::HistoryList))
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(shown, ["Alt + ↓"]);
    }

    #[test]
    fn help_describes_every_menu_key() {
        let described: Vec<Action> = crate::help::sections()
            .into_iter()
            .flat_map(|(_, rows)| rows.into_iter().filter_map(|(_, _, a)| a))
            .collect();
        for (_, _, act) in table() {
            if let MenuAct::Key(a) = act {
                assert!(described.contains(&a), "{a:?} missing in help");
            }
        }
    }

    #[test]
    fn every_menu_key_has_an_item() {
        let all = Checks {
            classic: true,
            ..Checks::default()
        };
        fn acts(items: Vec<Item>, out: &mut Vec<MenuAct>) {
            for i in items {
                match i {
                    menu::Item::Button(_, _, a) | menu::Item::CheckBox(_, _, _, a) => out.push(a),
                    menu::Item::Folder(_, sub) => acts(sub, out),
                    _ => {}
                }
            }
        }
        let mut items = Vec::new();
        for (_, menu) in menus(all, Lock::default()) {
            acts(menu, &mut items);
        }
        let missing: Vec<_> = (table().into_iter())
            .map(|(_, _, a)| a)
            // Enter: the context menu's «Open».
            .filter(|a| *a != MenuAct::Open && !items.contains(a))
            .collect();
        assert!(missing.is_empty(), "no menu item: {missing:?}");
    }

    #[test]
    fn the_frame_item_only_in_the_classic_look() {
        let has_frame = |classic| {
            menus(
                Checks {
                    classic,
                    ..Checks::default()
                },
                Lock::default(),
            )
            .into_iter()
            .flat_map(|(_, items)| items)
            .any(|i| {
                matches!(
                    i,
                    menu::Item::CheckBox(_, _, _, MenuAct::Key(Action::ToggleActiveFrame))
                )
            })
        };
        assert!(has_frame(true) && !has_frame(false));
    }

    #[test]
    fn every_item_shows_a_key() {
        let binds = key_binds();
        let all = Checks {
            classic: true,
            ..Checks::default()
        };
        for (title, items) in menus(all, Lock::default()) {
            for item in items {
                let act = match item {
                    menu::Item::Button(_, _, a) | menu::Item::CheckBox(_, _, _, a) => a,
                    _ => continue,
                };
                // No key, as in TC.
                if matches!(
                    act,
                    MenuAct::Key(
                        Action::About
                            | Action::Donate
                            | Action::TabCopyOther
                            | Action::TabMoveOther
                            | Action::TabLock
                            | Action::TabRoam
                            | Action::TabRename
                            | Action::ResetColumns
                            | Action::ToggleDividerLine
                            | Action::Vis(_)
                            | Action::Split50
                            | Action::Wide
                            | Action::VerticalPanels
                            | Action::ToggleCol(_)
                            | Action::ViewMode(_)
                            | Action::SetDate
                            | Action::TransferLeft
                            | Action::TransferRight
                            | Action::FocusLeft
                            | Action::FocusRight
                            | Action::FocusCmdLine
                            | Action::SaveViewMode
                            | Action::ToggleHumanSizes
                            | Action::ToggleActiveFrame
                            | Action::ConfigureToolbar
                            | Action::ConfigureStartMenu
                            | Action::ConfigureColors
                            | Action::ConfigureKeys
                            | Action::OpenTerminal
                            | Action::PickTerminal
                            | Action::PickShell
                            | Action::ChecksumCreate
                            | Action::ChecksumVerify
                            | Action::EditPath
                            | Action::OpenTrash
                            | Action::ArchiveEncoding
                            | Action::SyncChangeDir
                            | Action::MarkName
                            | Action::UnmarkName
                            | Action::MarkNameExt
                            | Action::UnmarkNameExt
                            | Action::SaveMarks
                            | Action::SaveMarksToFile
                            | Action::LoadMarksFromFile
                            | Action::LoadMarksFromClip
                            | Action::NextMarked
                            | Action::PrevMarked
                            | Action::ShowOnlyMarked
                            | Action::NewTabOther
                            | Action::NewFromTemplate
                            | Action::SplitFile
                            | Action::CombineFiles
                            | Action::CloseDuplicateTabs
                            | Action::CloseAllTabs
                            | Action::LockAllTabs
                            | Action::RoamAllTabs
                            | Action::UnlockAllTabs
                            | Action::GoToLockedDir
                            | Action::CopySrcPath
                            | Action::CopyTrgPath
                            | Action::CopyDirsOfFiles
                            | Action::CopyDetails
                            | Action::CopyDetailsHdr
                            | Action::SaveDetails
                            | Action::SaveDetailsHdr
                            | Action::MatchSrc
                            | Action::Restore
                            | Action::EmptyTrash
                    )
                ) {
                    continue;
                }
                assert!(binds.values().any(|b| *b == act), "{title}: {act:?}");
            }
        }
    }
}
