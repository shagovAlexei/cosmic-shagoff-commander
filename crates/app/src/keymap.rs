//! TC key bindings → `Action`. One table; the F-key buttons dispatch the same actions.

use cosmic::iced::keyboard::{Key, Modifiers, key::Physical};
use shagoff_core::lister::{Encoding, Mode};
use shagoff_core::sort::SortKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SwitchPane,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Parent,
    Root,
    Reload,
    Sort(SortKey),
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    Mark,
    MarkDown,
    MarkUp,
    SelectGroup,
    UnselectGroup,
    Invert,
    SelectAll,
    UnselectAll,
    Copy,
    /// Shift+F5: copy the file under the cursor in its own dir under a new name.
    CopySame,
    Move,
    Rename,
    Mkdir,
    Delete,
    DeletePermanent,
    View,
    Edit,
    ToggleHidden,
    /// Alt+F1 / Alt+F2: drive list for pane 0 / 1.
    Drives(usize),
    /// Alt+symbol: quick search starting with this character.
    QuickSearch(char),
    /// Ctrl+S: quick filter field.
    QuickFilter,
    /// Alt+← / Alt+→: step through the tab's directory history.
    HistoryBack,
    HistoryForward,
    /// Alt+↓: the tab's recent directories as a list.
    HistoryList,
    /// Ctrl+D: favourite directories.
    Hotlist,
    /// Ctrl+U: swap the two panels.
    SwapPanes,
    /// Ctrl+C / Ctrl+X: put the targets on the system clipboard; Ctrl+V: paste files from it.
    ClipCopy,
    ClipCut,
    ClipPaste,
    MultiRename,
    /// Alt+F5 / Alt+F9: pack the targets / unpack the archives among them.
    Pack,
    Unpack,
    /// Alt+F7: find files.
    FindFiles,
    /// Shift+F2: mark what differs between the two panels.
    CompareLists,
    /// Ctrl+Shift+S: synchronize the two panels' dirs.
    SyncDirs,
    /// Ctrl+Shift+D: compare two files by content.
    CompareFiles,
    /// F1: help in the side drawer.
    Help,
    /// Ctrl+,: settings in the side drawer.
    Settings,
    /// Menu only: about the program.
    About,
    /// Menu only: support the project.
    Donate,
    /// F9 / F10: names / full paths of the targets to the clipboard as text.
    CopyNames,
    CopyPaths,
    /// Ctrl+F: connect to a network location (TC: FTP connect).
    Connect,
    /// Ctrl+Shift+F: unmount / eject the drive of the active panel (TC: FTP disconnect).
    Disconnect,
    /// Command line: name / full path under the cursor (Ctrl+Enter / Ctrl+Shift+Enter), the
    /// panel's path (Ctrl+P), the previous command (Ctrl+E), the history list (Alt+F8).
    CmdName,
    CmdPath,
    CmdCwd,
    CmdPrevious,
    CmdHistory,
    /// Ctrl+↑ / Ctrl+Shift+↑: the dir under the cursor in a new tab here / in the other panel.
    TabOpen,
    TabOpenOther,
    /// Menu only (TC: tab context menu): the active tab copied / moved to the other panel.
    TabCopyOther,
    TabMoveOther,
    /// Menu only: lock / unlock, own caption.
    TabLock,
    TabRename,
    /// Ctrl+Shift+W: close the panel's other tabs but the locked ones.
    CloseOtherTabs,
    /// ←/→: the next column in Brief view; nothing in Full.
    Left,
    Right,
    /// Ctrl+F1 / Ctrl+F2: Brief / Full view of the active tab.
    ViewBrief,
    ViewFull,
}

pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
    use cosmic::iced::keyboard::key::{Code, Named};
    if mods.logo() {
        return None;
    }
    if mods.alt() {
        // Alt+F1/F2: drives; Alt+symbol: quick search (TC). Other Alt combos belong to the compositor.
        return match key {
            Key::Named(Named::F1) if mods == Modifiers::ALT => Some(Action::Drives(0)),
            Key::Named(Named::F2) if mods == Modifiers::ALT => Some(Action::Drives(1)),
            Key::Named(Named::F5) if mods == Modifiers::ALT => Some(Action::Pack),
            Key::Named(Named::F7) if mods == Modifiers::ALT => Some(Action::FindFiles),
            Key::Named(Named::F8) if mods == Modifiers::ALT => Some(Action::CmdHistory),
            Key::Named(Named::F9) if mods == Modifiers::ALT => Some(Action::Unpack),
            Key::Named(Named::ArrowLeft) if mods == Modifiers::ALT => Some(Action::HistoryBack),
            Key::Named(Named::ArrowRight) if mods == Modifiers::ALT => Some(Action::HistoryForward),
            Key::Named(Named::ArrowDown) if mods == Modifiers::ALT => Some(Action::HistoryList),
            Key::Character(s) if mods == Modifiers::ALT => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => Some(Action::QuickSearch(c)),
                    _ => None,
                }
            }
            _ => None,
        };
    }
    let (ctrl, shift) = (mods.control(), mods.shift());
    if let Key::Named(n) = key {
        return Some(match (n, ctrl, shift) {
            (Named::Tab, false, false) => Action::SwitchPane,
            (Named::Tab, true, false) => Action::NextTab,
            (Named::Tab, true, true) => Action::PrevTab,
            (Named::ArrowUp, false, false) => Action::Up,
            (Named::ArrowUp, true, false) => Action::TabOpen,
            (Named::ArrowUp, true, true) => Action::TabOpenOther,
            (Named::ArrowDown, false, false) => Action::Down,
            (Named::ArrowLeft, false, false) => Action::Left,
            (Named::ArrowRight, false, false) => Action::Right,
            (Named::F1, true, false) => Action::ViewBrief,
            (Named::F2, true, false) => Action::ViewFull,
            (Named::PageUp, false, false) => Action::PageUp,
            (Named::PageDown, false, false) => Action::PageDown,
            (Named::Home, false, false) => Action::Home,
            (Named::End, false, false) => Action::End,
            (Named::Enter, false, false) => Action::Enter,
            (Named::Enter, true, false) => Action::CmdName,
            (Named::Enter, true, true) => Action::CmdPath,
            (Named::Backspace, false, false) | (Named::PageUp, true, false) => Action::Parent,
            // TC: Ctrl+PgDn enters the dir or archive under the cursor.
            (Named::PageDown, true, false) => Action::Enter,
            (Named::F3, true, false) => Action::Sort(SortKey::Name),
            (Named::F4, true, false) => Action::Sort(SortKey::Ext),
            (Named::F5, true, false) => Action::Sort(SortKey::Date),
            (Named::F6, true, false) => Action::Sort(SortKey::Size),
            (Named::Insert, false, false) | (Named::ArrowDown, false, true) => Action::MarkDown,
            (Named::ArrowUp, false, true) => Action::MarkUp,
            (Named::F1, false, false) => Action::Help,
            (Named::F9, false, false) => Action::CopyNames,
            (Named::F10, false, false) => Action::CopyPaths,
            (Named::F11, false, false) => Action::CompareFiles,
            (Named::F3, false, false) => Action::View,
            (Named::F4, false, false) => Action::Edit,
            (Named::F5, false, false) => Action::Copy,
            (Named::F5, false, true) => Action::CopySame,
            (Named::F6, false, false) => Action::Move,
            (Named::F6, false, true) | (Named::F2, false, false) => Action::Rename,
            (Named::F2, false, true) => Action::CompareLists,
            (Named::F7, false, false) => Action::Mkdir,
            (Named::F8 | Named::Delete, false, false) => Action::Delete,
            (Named::F8 | Named::Delete, false, true) => Action::DeletePermanent,
            _ => return None,
        });
    }
    // Letters, Space and numpad by physical key so they work in any layout.
    match (physical, ctrl, shift) {
        (Physical::Code(Code::KeyR), true, false) => Some(Action::Reload),
        (Physical::Code(Code::Backslash), true, false) => Some(Action::Root),
        (Physical::Code(Code::KeyT), true, false) => Some(Action::NewTab),
        (Physical::Code(Code::KeyW), true, false) => Some(Action::CloseTab),
        (Physical::Code(Code::KeyW), true, true) => Some(Action::CloseOtherTabs),
        (Physical::Code(Code::KeyH), true, false) => Some(Action::ToggleHidden),
        (Physical::Code(Code::KeyS), true, false) => Some(Action::QuickFilter),
        (Physical::Code(Code::KeyS), true, true) => Some(Action::SyncDirs),
        (Physical::Code(Code::KeyD), true, true) => Some(Action::CompareFiles),
        (Physical::Code(Code::KeyD), true, false) => Some(Action::Hotlist),
        (Physical::Code(Code::KeyU), true, false) => Some(Action::SwapPanes),
        (Physical::Code(Code::Comma), true, false) => Some(Action::Settings),
        (Physical::Code(Code::KeyF), true, false) => Some(Action::Connect),
        (Physical::Code(Code::KeyF), true, true) => Some(Action::Disconnect),
        (Physical::Code(Code::KeyC), true, false) => Some(Action::ClipCopy),
        (Physical::Code(Code::KeyX), true, false) => Some(Action::ClipCut),
        (Physical::Code(Code::KeyV), true, false) => Some(Action::ClipPaste),
        (Physical::Code(Code::KeyM), true, false) => Some(Action::MultiRename),
        (Physical::Code(Code::KeyP), true, false) => Some(Action::CmdCwd),
        (Physical::Code(Code::KeyE), true, false) => Some(Action::CmdPrevious),
        (Physical::Code(Code::Space), false, false) => Some(Action::Mark),
        (Physical::Code(Code::NumpadAdd), false, false) => Some(Action::SelectGroup),
        (Physical::Code(Code::NumpadSubtract), false, false) => Some(Action::UnselectGroup),
        (Physical::Code(Code::NumpadMultiply), false, false) => Some(Action::Invert),
        (Physical::Code(Code::NumpadAdd | Code::KeyA), true, false) => Some(Action::SelectAll),
        (Physical::Code(Code::NumpadSubtract), true, false) => Some(Action::UnselectAll),
        _ => None,
    }
}

/// Keys the viewer (F3) adds; looked at only while it is open, after `action` found nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListerKey {
    Mode(Mode),
    Next,
    Prev,
    Close,
    FindPrev,
    /// A (ANSI = cp1251), S (DOS = cp866), K (KOI8-R), 8 (UTF-8 / BOM).
    Encoding(Encoding),
    /// W: wrap long lines.
    Wrap,
}

pub fn lister_key(key: &Key, physical: Physical, mods: Modifiers) -> Option<ListerKey> {
    use cosmic::iced::keyboard::key::{Code, Named};
    match key {
        Key::Named(Named::F3) if mods == Modifiers::SHIFT => return Some(ListerKey::FindPrev),
        _ if !mods.is_empty() => return None,
        _ => {}
    }
    // TC: 1 text, 3 hex, 4 multimedia; N / P next / previous file; Q closes.
    Some(match physical {
        Physical::Code(Code::Digit1 | Code::Numpad1) => ListerKey::Mode(Mode::Text),
        Physical::Code(Code::Digit3 | Code::Numpad3) => ListerKey::Mode(Mode::Hex),
        Physical::Code(Code::Digit4 | Code::Numpad4) => ListerKey::Mode(Mode::Image),
        Physical::Code(Code::KeyN) => ListerKey::Next,
        Physical::Code(Code::KeyP) => ListerKey::Prev,
        Physical::Code(Code::KeyQ) => ListerKey::Close,
        Physical::Code(Code::KeyW) => ListerKey::Wrap,
        Physical::Code(Code::KeyA) => ListerKey::Encoding(Encoding::Cp1251),
        Physical::Code(Code::KeyS) => ListerKey::Encoding(Encoding::Cp866),
        Physical::Code(Code::KeyK) => ListerKey::Encoding(Encoding::Koi8r),
        Physical::Code(Code::Digit8 | Code::Numpad8) => ListerKey::Encoding(Encoding::Auto),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::keyboard::key::{Code, Named, NativeCode};

    const NONE: Modifiers = Modifiers::empty();
    const CTRL: Modifiers = Modifiers::CTRL;

    fn named(n: Named, mods: Modifiers) -> Option<Action> {
        action(
            &Key::Named(n),
            Physical::Unidentified(NativeCode::Unidentified),
            mods,
        )
    }
    fn chr(c: &str, code: Code, mods: Modifiers) -> Option<Action> {
        action(&Key::Character(c.into()), Physical::Code(code), mods)
    }

    #[test]
    fn navigation_keys() {
        assert_eq!(named(Named::Tab, NONE), Some(Action::SwitchPane));
        assert_eq!(named(Named::ArrowUp, NONE), Some(Action::Up));
        assert_eq!(named(Named::ArrowDown, NONE), Some(Action::Down));
        assert_eq!(named(Named::PageUp, NONE), Some(Action::PageUp));
        assert_eq!(named(Named::PageDown, NONE), Some(Action::PageDown));
        assert_eq!(named(Named::Home, NONE), Some(Action::Home));
        assert_eq!(named(Named::End, NONE), Some(Action::End));
        assert_eq!(named(Named::ArrowLeft, NONE), Some(Action::Left));
        assert_eq!(named(Named::ArrowRight, NONE), Some(Action::Right));
        assert_eq!(named(Named::F1, CTRL), Some(Action::ViewBrief));
        assert_eq!(named(Named::F2, CTRL), Some(Action::ViewFull));
        assert_eq!(named(Named::Enter, NONE), Some(Action::Enter));
        assert_eq!(named(Named::Backspace, NONE), Some(Action::Parent));
        assert_eq!(named(Named::PageUp, CTRL), Some(Action::Parent));
    }

    #[test]
    fn f9_f10_copy_names_f11_compares() {
        assert_eq!(named(Named::F9, NONE), Some(Action::CopyNames));
        assert_eq!(named(Named::F5, Modifiers::SHIFT), Some(Action::CopySame));
        assert_eq!(named(Named::F10, NONE), Some(Action::CopyPaths));
        assert_eq!(named(Named::F11, NONE), Some(Action::CompareFiles));
    }

    #[test]
    fn f1_help_ctrl_comma_settings() {
        assert_eq!(named(Named::F1, NONE), Some(Action::Help));
        assert_eq!(chr(",", Code::Comma, CTRL), Some(Action::Settings));
        assert_eq!(chr("б", Code::Comma, CTRL), Some(Action::Settings)); // Russian layout
    }

    #[test]
    fn ctrl_f_connects_ctrl_shift_f_disconnects() {
        let shift = CTRL | Modifiers::SHIFT;
        assert_eq!(chr("f", Code::KeyF, CTRL), Some(Action::Connect));
        assert_eq!(chr("а", Code::KeyF, CTRL), Some(Action::Connect)); // Russian layout
        assert_eq!(chr("F", Code::KeyF, shift), Some(Action::Disconnect));
    }

    #[test]
    fn ctrl_f_keys_sort() {
        assert_eq!(named(Named::F3, CTRL), Some(Action::Sort(SortKey::Name)));
        assert_eq!(named(Named::F4, CTRL), Some(Action::Sort(SortKey::Ext)));
        assert_eq!(named(Named::F5, CTRL), Some(Action::Sort(SortKey::Date)));
        assert_eq!(named(Named::F6, CTRL), Some(Action::Sort(SortKey::Size)));
    }

    #[test]
    fn ctrl_letters_use_physical_key() {
        assert_eq!(chr("r", Code::KeyR, CTRL), Some(Action::Reload));
        assert_eq!(chr("к", Code::KeyR, CTRL), Some(Action::Reload)); // Russian layout
        assert_eq!(chr("\\", Code::Backslash, CTRL), Some(Action::Root));
        assert_eq!(chr("ё", Code::Backslash, CTRL), Some(Action::Root));
        assert_eq!(chr("r", Code::KeyR, NONE), None);
    }

    #[test]
    fn ctrl_c_x_v_clipboard() {
        assert_eq!(chr("c", Code::KeyC, CTRL), Some(Action::ClipCopy));
        assert_eq!(chr("с", Code::KeyC, CTRL), Some(Action::ClipCopy)); // Russian layout
        assert_eq!(chr("x", Code::KeyX, CTRL), Some(Action::ClipCut));
        assert_eq!(chr("ч", Code::KeyX, CTRL), Some(Action::ClipCut));
        assert_eq!(chr("v", Code::KeyV, CTRL), Some(Action::ClipPaste));
        assert_eq!(chr("м", Code::KeyV, CTRL), Some(Action::ClipPaste));
        assert_eq!(chr("c", Code::KeyC, NONE), None);
    }

    #[test]
    fn ctrl_m_multi_rename() {
        assert_eq!(chr("m", Code::KeyM, CTRL), Some(Action::MultiRename));
        assert_eq!(chr("ь", Code::KeyM, CTRL), Some(Action::MultiRename)); // Russian layout
        assert_eq!(chr("m", Code::KeyM, NONE), None);
    }

    #[test]
    fn tab_keys() {
        const CTRL_SHIFT: Modifiers = Modifiers::CTRL.union(Modifiers::SHIFT);
        assert_eq!(named(Named::Tab, CTRL), Some(Action::NextTab));
        assert_eq!(named(Named::Tab, CTRL_SHIFT), Some(Action::PrevTab));
        assert_eq!(chr("t", Code::KeyT, CTRL), Some(Action::NewTab));
        assert_eq!(chr("е", Code::KeyT, CTRL), Some(Action::NewTab)); // Russian layout
        assert_eq!(chr("w", Code::KeyW, CTRL), Some(Action::CloseTab));
        assert_eq!(chr("ц", Code::KeyW, CTRL), Some(Action::CloseTab));
    }

    #[test]
    fn modifiers_must_match_exactly() {
        const SHIFT: Modifiers = Modifiers::SHIFT;
        const ALT: Modifiers = Modifiers::ALT;
        assert_eq!(named(Named::Tab, SHIFT), None);
        assert_eq!(named(Named::Enter, ALT), None);
        assert_eq!(named(Named::PageDown, SHIFT), None);
        assert_eq!(named(Named::F3, CTRL.union(ALT)), None);
        assert_eq!(chr("R", Code::KeyR, CTRL.union(SHIFT)), None);
    }

    #[test]
    fn marking_keys() {
        const SHIFT: Modifiers = Modifiers::SHIFT;
        assert_eq!(named(Named::Insert, NONE), Some(Action::MarkDown));
        assert_eq!(chr(" ", Code::Space, NONE), Some(Action::Mark));
        assert_eq!(named(Named::ArrowDown, SHIFT), Some(Action::MarkDown));
        assert_eq!(named(Named::ArrowUp, SHIFT), Some(Action::MarkUp));
    }

    #[test]
    fn numpad_marking() {
        assert_eq!(chr("+", Code::NumpadAdd, NONE), Some(Action::SelectGroup));
        assert_eq!(
            chr("-", Code::NumpadSubtract, NONE),
            Some(Action::UnselectGroup)
        );
        assert_eq!(chr("*", Code::NumpadMultiply, NONE), Some(Action::Invert));
        assert_eq!(chr("+", Code::NumpadAdd, CTRL), Some(Action::SelectAll));
        assert_eq!(
            chr("-", Code::NumpadSubtract, CTRL),
            Some(Action::UnselectAll)
        );
        assert_eq!(chr("a", Code::KeyA, CTRL), Some(Action::SelectAll));
        assert_eq!(chr("ф", Code::KeyA, CTRL), Some(Action::SelectAll)); // Russian layout
    }

    #[test]
    fn main_keyboard_plus_is_not_numpad() {
        assert_eq!(chr("+", Code::Equal, Modifiers::SHIFT), None);
        assert_eq!(chr("-", Code::Minus, NONE), None);
    }

    #[test]
    fn operation_keys() {
        const SHIFT: Modifiers = Modifiers::SHIFT;
        assert_eq!(named(Named::F5, NONE), Some(Action::Copy));
        assert_eq!(named(Named::F6, NONE), Some(Action::Move));
        assert_eq!(named(Named::F6, SHIFT), Some(Action::Rename));
        assert_eq!(named(Named::F2, NONE), Some(Action::Rename));
        assert_eq!(named(Named::F7, NONE), Some(Action::Mkdir));
        assert_eq!(named(Named::F8, NONE), Some(Action::Delete));
        assert_eq!(named(Named::Delete, NONE), Some(Action::Delete));
        assert_eq!(named(Named::F8, SHIFT), Some(Action::DeletePermanent));
        assert_eq!(named(Named::Delete, SHIFT), Some(Action::DeletePermanent));
    }

    #[test]
    fn unbound_combinations() {
        assert_eq!(named(Named::ArrowDown, CTRL), None);
        assert_eq!(named(Named::F12, NONE), None);
    }

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
        assert_eq!(chr("a", Code::KeyA, ALT), Some(Action::QuickSearch('a')));
        assert_eq!(named(Named::F1, Modifiers::LOGO), None);
    }

    #[test]
    fn tab_more_keys() {
        assert_eq!(named(Named::ArrowUp, CTRL), Some(Action::TabOpen));
        assert_eq!(
            named(Named::ArrowUp, CTRL | Modifiers::SHIFT),
            Some(Action::TabOpenOther)
        );
        assert_eq!(
            chr("W", Code::KeyW, CTRL | Modifiers::SHIFT),
            Some(Action::CloseOtherTabs)
        );
    }

    #[test]
    fn command_line_keys() {
        assert_eq!(named(Named::Enter, CTRL), Some(Action::CmdName));
        assert_eq!(
            named(Named::Enter, CTRL | Modifiers::SHIFT),
            Some(Action::CmdPath)
        );
        assert_eq!(named(Named::F8, ALT), Some(Action::CmdHistory));
        assert_eq!(chr("з", Code::KeyP, CTRL), Some(Action::CmdCwd));
        assert_eq!(chr("e", Code::KeyE, CTRL), Some(Action::CmdPrevious));
        assert_eq!(named(Named::Enter, Modifiers::SHIFT), None); // the field's own Enter
    }

    #[test]
    fn escape_is_not_a_panel_key() {
        // Escape is routed to DialogCancel by app::route_event before the keymap.
        assert_eq!(named(Named::Escape, NONE), None);
    }

    #[test]
    fn alt_letter_is_quick_search() {
        assert_eq!(chr("d", Code::KeyD, ALT), Some(Action::QuickSearch('d')));
        assert_eq!(chr("в", Code::KeyD, ALT), Some(Action::QuickSearch('в')));
        assert_eq!(chr("1", Code::Digit1, ALT), Some(Action::QuickSearch('1')));
        assert_eq!(chr("D", Code::KeyD, ALT | Modifiers::SHIFT), None);
        assert_eq!(chr("d", Code::KeyD, ALT | CTRL), None);
        assert_eq!(named(Named::F1, ALT), Some(Action::Drives(0))); // still drives
    }

    #[test]
    fn ctrl_s_is_quick_filter() {
        assert_eq!(chr("s", Code::KeyS, CTRL), Some(Action::QuickFilter));
        assert_eq!(chr("ы", Code::KeyS, CTRL), Some(Action::QuickFilter));
    }

    #[test]
    fn alt_arrows_walk_history() {
        assert_eq!(named(Named::ArrowLeft, ALT), Some(Action::HistoryBack));
        assert_eq!(named(Named::ArrowRight, ALT), Some(Action::HistoryForward));
        assert_eq!(named(Named::ArrowDown, ALT), Some(Action::HistoryList));
        assert_eq!(named(Named::ArrowLeft, ALT | Modifiers::SHIFT), None);
    }

    #[test]
    fn ctrl_d_hotlist_ctrl_u_swap() {
        assert_eq!(chr("d", Code::KeyD, CTRL), Some(Action::Hotlist));
        assert_eq!(chr("в", Code::KeyD, CTRL), Some(Action::Hotlist));
        assert_eq!(chr("u", Code::KeyU, CTRL), Some(Action::SwapPanes));
        assert_eq!(chr("г", Code::KeyU, CTRL), Some(Action::SwapPanes));
    }

    #[test]
    fn alt_f5_f9_pack_unpack() {
        assert_eq!(named(Named::F5, ALT), Some(Action::Pack));
        assert_eq!(named(Named::F9, ALT), Some(Action::Unpack));
        assert_eq!(named(Named::F5, ALT | Modifiers::SHIFT), None);
    }

    #[test]
    fn ctrl_pgdn_enters() {
        assert_eq!(named(Named::PageDown, Modifiers::CTRL), Some(Action::Enter));
    }

    #[test]
    fn alt_f7_finds() {
        assert_eq!(named(Named::F7, Modifiers::ALT), Some(Action::FindFiles));
    }

    #[test]
    fn compare_and_sync_keys() {
        assert_eq!(
            named(Named::F2, Modifiers::SHIFT),
            Some(Action::CompareLists)
        );
        let s = action(
            &Key::Character("S".into()),
            Physical::Code(Code::KeyS),
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        assert_eq!(s, Some(Action::SyncDirs));
    }

    #[test]
    fn ctrl_shift_d_compares_files() {
        let d = action(
            &Key::Character("D".into()),
            Physical::Code(Code::KeyD),
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        assert_eq!(d, Some(Action::CompareFiles));
    }

    #[test]
    fn lister_keys_are_free_in_the_main_table() {
        let keys = [
            (
                Key::Character("1".into()),
                Code::Digit1,
                ListerKey::Mode(Mode::Text),
            ),
            (
                Key::Character("3".into()),
                Code::Digit3,
                ListerKey::Mode(Mode::Hex),
            ),
            (
                Key::Character("4".into()),
                Code::Digit4,
                ListerKey::Mode(Mode::Image),
            ),
            (Key::Character("т".into()), Code::KeyN, ListerKey::Next), // any layout
            (Key::Character("p".into()), Code::KeyP, ListerKey::Prev),
            (Key::Character("q".into()), Code::KeyQ, ListerKey::Close),
            (Key::Character("ц".into()), Code::KeyW, ListerKey::Wrap),
            (
                Key::Character("a".into()),
                Code::KeyA,
                ListerKey::Encoding(Encoding::Cp1251),
            ),
            (
                Key::Character("s".into()),
                Code::KeyS,
                ListerKey::Encoding(Encoding::Cp866),
            ),
            (
                Key::Character("k".into()),
                Code::KeyK,
                ListerKey::Encoding(Encoding::Koi8r),
            ),
            (
                Key::Character("8".into()),
                Code::Digit8,
                ListerKey::Encoding(Encoding::Auto),
            ),
        ];
        for (key, code, want) in keys {
            assert_eq!(action(&key, Physical::Code(code), NONE), None, "{want:?}");
            assert_eq!(lister_key(&key, Physical::Code(code), NONE), Some(want));
        }
        let f3 = Key::Named(Named::F3);
        let ph = Physical::Code(Code::F3);
        assert_eq!(action(&f3, ph, Modifiers::SHIFT), None);
        assert_eq!(
            lister_key(&f3, ph, Modifiers::SHIFT),
            Some(ListerKey::FindPrev)
        );
        assert_eq!(lister_key(&f3, ph, NONE), None); // F3 is View: find next
        let ctrl_n = lister_key(
            &Key::Character("n".into()),
            Physical::Code(Code::KeyN),
            CTRL,
        );
        assert_eq!(ctrl_n, None);
    }
}
