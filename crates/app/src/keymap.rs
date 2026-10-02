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
}

pub fn action(key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
    use cosmic::iced::keyboard::key::{Code, Named};
    if mods.alt() || mods.logo() {
        return None;
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
            _ => return None,
        });
    }
    // Letter shortcuts by physical key so they work in any layout.
    match (physical, ctrl, shift) {
        (Physical::Code(Code::KeyR), true, false) => Some(Action::Reload),
        (Physical::Code(Code::Backslash), true, false) => Some(Action::Root),
        (Physical::Code(Code::KeyT), true, false) => Some(Action::NewTab),
        (Physical::Code(Code::KeyW), true, false) => Some(Action::CloseTab),
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
        assert_eq!(named(Named::ArrowDown, SHIFT), None);
        assert_eq!(named(Named::F3, CTRL.union(ALT)), None);
        assert_eq!(chr("R", Code::KeyR, CTRL.union(SHIFT)), None);
    }

    #[test]
    fn unbound_combinations() {
        assert_eq!(named(Named::ArrowUp, CTRL), None);
        assert_eq!(named(Named::Escape, NONE), None);
    }
}
