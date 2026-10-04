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
    Exit,
}

impl menu::Action for MenuAct {
    type Message = Message;
    fn message(&self) -> Message {
        match *self {
            MenuAct::Key(a) => Message::Key(a),
            MenuAct::Exit => Message::Exit,
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
        (vec![], K::Named(Named::F3), a(Action::View)),
        (vec![], K::Named(Named::F4), a(Action::Edit)),
        (vec![], K::Named(Named::F5), a(Action::Copy)),
        (vec![Shift], K::Named(Named::F5), a(Action::CopySame)),
        (vec![], K::Named(Named::F6), a(Action::Move)),
        (vec![Shift], K::Named(Named::F6), a(Action::Rename)),
        (vec![], K::Named(Named::F7), a(Action::Mkdir)),
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
            vec![Ctrl],
            K::Num(Code::NumpadSubtract, "Num −"),
            a(Action::UnselectAll),
        ),
        (
            vec![],
            K::Num(Code::NumpadMultiply, "Num *"),
            a(Action::Invert),
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
        (
            vec![Ctrl],
            K::Letter(Code::KeyS, "s"),
            a(Action::QuickFilter),
        ),
        (vec![Ctrl], K::Letter(Code::KeyR, "r"), a(Action::Reload)),
        (vec![Ctrl], K::Letter(Code::KeyT, "t"), a(Action::NewTab)),
        (vec![Ctrl], K::Letter(Code::KeyW, "w"), a(Action::CloseTab)),
        (
            vec![Ctrl, Shift],
            K::Letter(Code::KeyW, "w"),
            a(Action::CloseOtherTabs),
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
        (vec![], K::Named(Named::F1), a(Action::Help)),
        (vec![Ctrl], K::Named(Named::F1), a(Action::ViewBrief)),
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
            K::Named(Named::F6),
            a(Action::Sort(SortKey::Size)),
        ),
    ]
}

fn key_binds() -> HashMap<KeyBind, MenuAct> {
    table()
        .into_iter()
        .map(|(modifiers, k, act)| {
            let key = match k {
                K::Named(n) => Key::Named(n),
                K::Letter(_, s) | K::Num(_, s) | K::Shown(_, s) => Key::Character(s.into()),
            };
            (KeyBind { modifiers, key }, act)
        })
        .collect()
}

type Item = menu::Item<MenuAct, String>;

/// (title, items) of every menu.
fn menus(show_hidden: bool, locked: bool) -> Vec<(String, Vec<Item>)> {
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
                b(fl!("menu-move"), Action::Move),
                b(fl!("menu-rename"), Action::Rename),
                b(fl!("menu-mkdir"), Action::Mkdir),
                b(fl!("menu-delete"), Action::Delete),
                b(fl!("menu-delete-permanent"), Action::DeletePermanent),
                menu::Item::Divider,
                b(fl!("menu-pack"), Action::Pack),
                b(fl!("menu-unpack"), Action::Unpack),
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
                menu::Item::Divider,
                b(fl!("menu-compare-lists"), Action::CompareLists),
                menu::Item::Divider,
                b(fl!("menu-copy-names"), Action::CopyNames),
                b(fl!("menu-copy-paths"), Action::CopyPaths),
            ],
        ),
        (
            fl!("menu-commands"),
            vec![
                b(fl!("menu-find"), Action::FindFiles),
                b(fl!("menu-sync"), Action::SyncDirs),
                menu::Item::Divider,
                b(fl!("menu-hotlist"), Action::Hotlist),
                b(fl!("menu-history"), Action::HistoryList),
                b(fl!("menu-swap"), Action::SwapPanes),
                menu::Item::Divider,
                b(fl!("menu-filter"), Action::QuickFilter),
                b(fl!("menu-reload"), Action::Reload),
            ],
        ),
        (
            fl!("menu-tabs"),
            vec![
                b(fl!("menu-new-tab"), Action::NewTab),
                b(fl!("menu-tab-open"), Action::TabOpen),
                b(fl!("menu-tab-open-other"), Action::TabOpenOther),
                menu::Item::Divider,
                b(fl!("menu-tab-copy-other"), Action::TabCopyOther),
                b(fl!("menu-tab-move-other"), Action::TabMoveOther),
                menu::Item::Divider,
                menu::Item::CheckBox(
                    fl!("menu-tab-lock"),
                    None,
                    locked,
                    MenuAct::Key(Action::TabLock),
                ),
                b(fl!("menu-tab-rename"), Action::TabRename),
                menu::Item::Divider,
                b(fl!("menu-close-tab"), Action::CloseTab),
                b(fl!("menu-close-other-tabs"), Action::CloseOtherTabs),
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
                    show_hidden,
                    MenuAct::Key(Action::ToggleHidden),
                ),
                menu::Item::Divider,
                b(fl!("menu-brief"), Action::ViewBrief),
                b(fl!("menu-full"), Action::ViewFull),
                menu::Item::Divider,
                sort(fl!("menu-sort-name"), SortKey::Name),
                sort(fl!("menu-sort-ext"), SortKey::Ext),
                sort(fl!("menu-sort-date"), SortKey::Date),
                sort(fl!("menu-sort-size"), SortKey::Size),
            ],
        ),
        (
            fl!("menu-config"),
            vec![b(fl!("menu-settings"), Action::Settings)],
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

pub fn bar(show_hidden: bool, locked: bool) -> Element<'static, Message> {
    let binds = key_binds();
    let roots = menus(show_hidden, locked)
        .into_iter()
        .map(|(title, items)| {
            menu::Tree::with_children(Element::from(menu::root(title)), menu::items(&binds, items))
        })
        .collect();
    menu::bar(roots)
        .item_height(ItemHeight::Dynamic(40))
        .item_width(ItemWidth::Uniform(320))
        .spacing(4.0)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap;
    use cosmic::iced::keyboard::Modifiers;
    use cosmic::iced::keyboard::key::Physical;

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
    fn every_item_shows_a_key() {
        let binds = key_binds();
        for (title, items) in menus(false, false) {
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
                            | Action::TabRename
                    )
                ) {
                    continue;
                }
                assert!(binds.values().any(|b| *b == act), "{title}: {act:?}");
            }
        }
    }
}
