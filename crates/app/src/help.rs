//! F1: what the program is and every key, in sections.

use crate::fl;
use crate::keymap::Action;
use shagoff_core::sort::SortKey;

/// (keys as shown, what they do, the action — for the "every menu key is here" test).
pub type Row = (&'static str, String, Option<Action>);

pub fn sections() -> Vec<(String, Vec<Row>)> {
    let r = |keys, what: String, a: Action| (keys, what, Some(a));
    let o = |keys, what: String| (keys, what, None);
    vec![
        (
            fl!("help-nav"),
            vec![
                o("Tab", fl!("help-tab")),
                o("↑ ↓ PgUp PgDn Home End", fl!("help-cursor")),
                o("← →", fl!("help-brief-cols")),
                r("Enter, Ctrl+PgDn", fl!("help-enter"), Action::Enter),
                o("Backspace, Ctrl+PgUp", fl!("help-parent")),
                o("Ctrl+\\", fl!("help-root")),
                o("Alt+← / Alt+→", fl!("help-history-step")),
                r("Alt+↓", fl!("menu-history"), Action::HistoryList),
                r("Ctrl+D", fl!("menu-hotlist"), Action::Hotlist),
                r("Ctrl+U", fl!("menu-swap"), Action::SwapPanes),
                o("Alt+…", fl!("help-quick-search")),
                r("Ctrl+S", fl!("menu-filter"), Action::QuickFilter),
                r("Ctrl+R", fl!("menu-reload"), Action::Reload),
            ],
        ),
        (
            fl!("help-marks"),
            vec![
                o("Insert, Shift+↓", fl!("help-mark-down")),
                o("Shift+↑", fl!("help-mark-up")),
                o("Space", fl!("help-mark")),
                r("Num +", fl!("menu-select-group"), Action::SelectGroup),
                r("Num −", fl!("menu-unselect-group"), Action::UnselectGroup),
                r(
                    "Ctrl+A, Ctrl+Num +",
                    fl!("menu-select-all"),
                    Action::SelectAll,
                ),
                r("Ctrl+Num −", fl!("menu-unselect-all"), Action::UnselectAll),
                r("Num *", fl!("menu-invert"), Action::Invert),
                r("Shift+F2", fl!("help-compare-lists"), Action::CompareLists),
            ],
        ),
        (
            fl!("help-files"),
            vec![
                r("F3", fl!("menu-view"), Action::View),
                r("F4", fl!("menu-edit"), Action::Edit),
                r("F5", fl!("menu-copy"), Action::Copy),
                r("Shift+F5", fl!("menu-copy-same"), Action::CopySame),
                r("Alt+Enter", fl!("menu-properties"), Action::Properties),
                r("Alt+Shift+Enter", fl!("menu-count-dirs"), Action::CountDirs),
                r("F6", fl!("menu-move"), Action::Move),
                r("Shift+F6, F2", fl!("menu-rename"), Action::Rename),
                r("F7", fl!("menu-mkdir"), Action::Mkdir),
                r("Shift+F4", fl!("menu-new-file"), Action::NewFile),
                r("F8, Delete", fl!("help-delete"), Action::Delete),
                r(
                    "Shift+F8, Shift+Delete",
                    fl!("menu-delete-permanent"),
                    Action::DeletePermanent,
                ),
                r("Ctrl+M", fl!("menu-multi-rename"), Action::MultiRename),
                r("Ctrl+C", fl!("menu-clip-copy"), Action::ClipCopy),
                r("Ctrl+X", fl!("menu-clip-cut"), Action::ClipCut),
                r("Ctrl+V", fl!("menu-clip-paste"), Action::ClipPaste),
                r(
                    "F11, Ctrl+Shift+D",
                    fl!("menu-compare-files"),
                    Action::CompareFiles,
                ),
                r("F9", fl!("menu-copy-names"), Action::CopyNames),
                r("F10", fl!("menu-copy-paths"), Action::CopyPaths),
            ],
        ),
        (
            fl!("help-archives"),
            vec![
                r("Alt+F5", fl!("menu-pack"), Action::Pack),
                r("Alt+F9", fl!("menu-unpack"), Action::Unpack),
                o("Enter", fl!("help-archive-enter")),
            ],
        ),
        (
            fl!("help-search"),
            vec![
                r("Alt+F7", fl!("menu-find"), Action::FindFiles),
                r("Ctrl+Shift+S", fl!("menu-sync"), Action::SyncDirs),
            ],
        ),
        (
            fl!("help-tabs"),
            vec![
                r("Ctrl+T", fl!("menu-new-tab"), Action::NewTab),
                r("Ctrl+W", fl!("menu-close-tab"), Action::CloseTab),
                r(
                    "Ctrl+Shift+W",
                    fl!("menu-close-other-tabs"),
                    Action::CloseOtherTabs,
                ),
                r("Ctrl+↑", fl!("menu-tab-open"), Action::TabOpen),
                r(
                    "Ctrl+Shift+↑",
                    fl!("menu-tab-open-other"),
                    Action::TabOpenOther,
                ),
                o("Ctrl+Tab / Ctrl+Shift+Tab", fl!("help-next-tab")),
            ],
        ),
        (
            fl!("help-drives"),
            vec![
                r("Alt+F1", fl!("menu-drive-left"), Action::Drives(0)),
                r("Alt+F2", fl!("menu-drive-right"), Action::Drives(1)),
                r("Ctrl+F", fl!("menu-connect"), Action::Connect),
                r("Ctrl+Shift+F", fl!("menu-disconnect"), Action::Disconnect),
            ],
        ),
        (
            fl!("help-show"),
            vec![
                r("Ctrl+H", fl!("menu-hidden"), Action::ToggleHidden),
                r("Ctrl+F1", fl!("menu-brief"), Action::ViewBrief),
                r("Ctrl+F2", fl!("menu-full"), Action::ViewFull),
                r(
                    "Ctrl+F3",
                    fl!("menu-sort-name"),
                    Action::Sort(SortKey::Name),
                ),
                r("Ctrl+F4", fl!("menu-sort-ext"), Action::Sort(SortKey::Ext)),
                r(
                    "Ctrl+F5",
                    fl!("menu-sort-date"),
                    Action::Sort(SortKey::Date),
                ),
                r(
                    "Ctrl+F6",
                    fl!("menu-sort-size"),
                    Action::Sort(SortKey::Size),
                ),
            ],
        ),
        (
            fl!("help-mouse"),
            vec![
                o("LMB", fl!("help-mouse-click")),
                o("Ctrl+LMB", fl!("help-mouse-mark")),
                o("LMB ×2", fl!("help-mouse-open")),
                o("RMB", fl!("help-mouse-menu")),
                o("RMB ↓", fl!("help-mouse-menu-dir")),
                o("RMB / MMB", fl!("help-mouse-tab")),
            ],
        ),
        (
            fl!("help-lister"),
            vec![
                o("↑ ↓ PgUp PgDn Home End, ← →", fl!("help-lister-scroll")),
                o("1 / 3 / 4", fl!("help-lister-modes")),
                o("W", fl!("help-lister-wrap")),
                o("A / S / K / 8", fl!("help-lister-encoding")),
                o("N / P", fl!("help-lister-step")),
                o("F7", fl!("help-lister-find")),
                o("F3 / Shift+F3", fl!("help-lister-again")),
                o("LMB, Shift+LMB", fl!("help-lister-select")),
                o("Ctrl+A / Ctrl+C", fl!("help-lister-copy")),
                o("Esc, Q", fl!("help-lister-close")),
            ],
        ),
        (
            fl!("help-cmdline"),
            vec![
                o("a…z, 0…9, …", fl!("help-cmd-type")),
                o("Enter / Shift+Enter", fl!("help-cmd-run")),
                r("Ctrl+Enter", fl!("help-cmd-name"), Action::CmdName),
                r("Ctrl+Shift+Enter", fl!("help-cmd-path"), Action::CmdPath),
                r("Ctrl+P", fl!("help-cmd-cwd"), Action::CmdCwd),
                r("Ctrl+E", fl!("help-cmd-previous"), Action::CmdPrevious),
                r("Alt+F8", fl!("cmd-history"), Action::CmdHistory),
            ],
        ),
        (
            fl!("help-window"),
            vec![
                r("F1", fl!("menu-help"), Action::Help),
                r("Ctrl+,", fl!("menu-settings"), Action::Settings),
                o("Escape", fl!("help-escape")),
                o("Alt+F4", fl!("menu-exit")),
            ],
        ),
    ]
}
