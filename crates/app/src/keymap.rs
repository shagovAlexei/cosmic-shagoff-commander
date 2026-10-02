//! TC key bindings → `Action`. One table; the F-key buttons dispatch the same actions.

use cosmic::iced::keyboard::{Key, Modifiers, key::Physical};
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
}

pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
    use cosmic::iced::keyboard::key::{Code, Named};
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
    let (ctrl, shift) = (mods.control(), mods.shift());
    if let Key::Named(n) = key {
        return Some(match (n, ctrl, shift) {
            (Named::Tab, false, false) => Action::SwitchPane,
            (Named::Tab, true, false) => Action::NextTab,
            (Named::Tab, true, true) => Action::PrevTab,
            (Named::ArrowUp, false, false) => Action::Up,
            (Named::ArrowDown, false, false) => Action::Down,
            (Named::PageUp, false, false) => Action::PageUp,
            (Named::PageDown, false, false) => Action::PageDown,
            (Named::Home, false, false) => Action::Home,
            (Named::End, false, false) => Action::End,
            (Named::Enter, false, false) => Action::Enter,
            (Named::Backspace, false, false) | (Named::PageUp, true, false) => Action::Parent,
            (Named::F3, true, false) => Action::Sort(SortKey::Name),
            (Named::F4, true, false) => Action::Sort(SortKey::Ext),
            (Named::F5, true, false) => Action::Sort(SortKey::Date),
            (Named::F6, true, false) => Action::Sort(SortKey::Size),
            (Named::Insert, false, false) | (Named::ArrowDown, false, true) => Action::MarkDown,
            (Named::ArrowUp, false, true) => Action::MarkUp,
            (Named::F3, false, false) => Action::View,
            (Named::F4, false, false) => Action::Edit,
            (Named::F5, false, false) => Action::Copy,
            (Named::F6, false, false) => Action::Move,
            (Named::F6, false, true) | (Named::F2, false, false) => Action::Rename,
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
        (Physical::Code(Code::KeyH), true, false) => Some(Action::ToggleHidden),
        (Physical::Code(Code::Space), false, false) => Some(Action::Mark),
        (Physical::Code(Code::NumpadAdd), false, false) => Some(Action::SelectGroup),
        (Physical::Code(Code::NumpadSubtract), false, false) => Some(Action::UnselectGroup),
        (Physical::Code(Code::NumpadMultiply), false, false) => Some(Action::Invert),
        (Physical::Code(Code::NumpadAdd | Code::KeyA), true, false) => Some(Action::SelectAll),
        (Physical::Code(Code::NumpadSubtract), true, false) => Some(Action::UnselectAll),
        _ => None,
    }
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
        assert_eq!(named(Named::Enter, NONE), Some(Action::Enter));
        assert_eq!(named(Named::Backspace, NONE), Some(Action::Parent));
        assert_eq!(named(Named::PageUp, CTRL), Some(Action::Parent));
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
        assert_eq!(named(Named::ArrowUp, CTRL), None);
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
        assert_eq!(chr("a", Code::KeyA, ALT), None);
        assert_eq!(named(Named::F1, Modifiers::LOGO), None);
    }

    #[test]
    fn escape_is_not_a_panel_key() {
        // Escape is routed to DialogCancel by app::route_event before the keymap.
        assert_eq!(named(Named::Escape, NONE), None);
    }
}
