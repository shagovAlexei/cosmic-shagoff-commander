//! TC key bindings → `Action`. One table; the F-key buttons dispatch the same actions.
#![allow(dead_code)]

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
}

pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
    use cosmic::iced::keyboard::key::{Code, Named};
    let ctrl = mods.control();
    if let Key::Named(n) = key {
        return Some(match (n, ctrl) {
            (Named::Tab, false) => Action::SwitchPane,
            (Named::ArrowUp, false) => Action::Up,
            (Named::ArrowDown, false) => Action::Down,
            (Named::PageUp, false) => Action::PageUp,
            (Named::PageDown, false) => Action::PageDown,
            (Named::Home, false) => Action::Home,
            (Named::End, false) => Action::End,
            (Named::Enter, false) => Action::Enter,
            (Named::Backspace, false) | (Named::PageUp, true) => Action::Parent,
            (Named::F3, true) => Action::Sort(SortKey::Name),
            (Named::F4, true) => Action::Sort(SortKey::Ext),
            (Named::F5, true) => Action::Sort(SortKey::Date),
            (Named::F6, true) => Action::Sort(SortKey::Size),
            _ => return None,
        });
    }
    // Letter shortcuts by physical key so they work in any layout.
    match (physical, ctrl) {
        (Physical::Code(Code::KeyR), true) => Some(Action::Reload),
        (Physical::Code(Code::Backslash), true) => Some(Action::Root),
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
        assert_eq!(named(Named::F3, NONE), None); // F3 = view, phase 6
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
    fn unbound_combinations() {
        assert_eq!(named(Named::ArrowUp, CTRL), None);
        assert_eq!(named(Named::Escape, NONE), None);
    }
}
