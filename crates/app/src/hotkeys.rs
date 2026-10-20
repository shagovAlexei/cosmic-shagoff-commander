//! Configuration → Hotkeys (TC «Redefine hotkeys»): the user's own keys, each set to a command —
//! any menu item or F1 row — or to nothing (the key off). Kept in `config.hotkeys` as
//! ("Ctrl+K", the action's name); `keymap` tries them before its table (`keymap::set_own`).

use crate::app::Message;
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Length};
use cosmic::{Element, theme, widget};
use shagoff_core::hotkey::Bind;

/// What a key can be set to: (where it is — menu → item, or help section: row — , action), each
/// action once, the menus' first.
pub fn commands() -> Vec<(String, Action)> {
    let mut out = crate::menu::commands();
    for (section, rows) in crate::help::sections() {
        for (_, what, a) in rows {
            if let Some(a) = a.filter(|a| !out.iter().any(|(_, b)| b == a)) {
                out.push((format!("{section}: {what}"), a));
            }
        }
    }
    out
}

/// How an action is kept in the config.
pub fn name(a: Action) -> String {
    format!("{a:?}")
}

/// The config's pairs as `keymap` takes them; a pair whose key or command is not known is left out.
pub fn resolve(list: &[(String, String)]) -> Vec<(Bind, Option<Action>)> {
    let cmds = commands();
    (list.iter())
        .filter_map(|(k, a)| {
            let act = match a.as_str() {
                "" => None,
                a => Some(cmds.iter().find(|(_, c)| name(*c) == a)?.1),
            };
            Some((Bind::parse(k)?, act))
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct KeyEdit {
    /// (key, action's name or "" = off), as in the config.
    pub list: Vec<(String, String)>,
    pub sel: Option<usize>,
    /// The selected row waits for its key.
    pub capturing: bool,
    cmds: Vec<(String, Action)>,
    /// The dropdown: «nothing (key off)», then `cmds`.
    labels: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum KeyMsg {
    Select(usize),
    /// A new row, waiting for its key.
    Add,
    /// The selected row's key again.
    Rebind,
    /// The key pressed while waiting (`Bind` as text).
    Captured(String),
    /// `labels[i]`: 0 = off.
    Command(usize),
    Delete,
    /// No own keys: the defaults only.
    Reset,
}

impl KeyEdit {
    pub fn new(list: Vec<(String, String)>) -> Self {
        let cmds = commands();
        let labels = std::iter::once(fl!("keys-off"))
            .chain(cmds.iter().map(|(l, _)| l.clone()))
            .collect();
        Self {
            sel: (!list.is_empty()).then_some(0),
            list,
            capturing: false,
            cmds,
            labels,
        }
    }

    pub fn update(&mut self, m: KeyMsg) {
        let sel = self.sel.filter(|&i| i < self.list.len());
        match m {
            KeyMsg::Select(i) => {
                self.sel = (i < self.list.len()).then_some(i);
                self.capturing = false;
            }
            KeyMsg::Add => {
                self.list.push((String::new(), String::new()));
                self.sel = Some(self.list.len() - 1);
                self.capturing = true;
            }
            KeyMsg::Rebind => self.capturing = sel.is_some(),
            KeyMsg::Captured(k) => {
                if let Some(i) = sel.filter(|_| self.capturing) {
                    self.list[i].0 = k.clone();
                    self.capturing = false;
                    // One row per key: an older one for the same key goes.
                    if let Some(j) = (0..self.list.len()).find(|&j| j != i && self.list[j].0 == k) {
                        self.list.remove(j);
                        self.sel = Some(if j < i { i - 1 } else { i });
                    }
                }
            }
            KeyMsg::Command(c) => {
                if let Some(i) = sel {
                    self.list[i].1 = match c {
                        0 => String::new(),
                        c => self
                            .cmds
                            .get(c - 1)
                            .map(|(_, a)| name(*a))
                            .unwrap_or_default(),
                    };
                }
            }
            KeyMsg::Delete => {
                if let Some(i) = sel {
                    self.list.remove(i);
                    self.sel = (!self.list.is_empty()).then(|| i.min(self.list.len() - 1));
                    self.capturing = false;
                }
            }
            KeyMsg::Reset => *self = Self::new(Vec::new()),
        }
    }

    /// What is kept: rows with a key.
    pub fn result(self) -> Vec<(String, String)> {
        self.list
            .into_iter()
            .filter(|(k, _)| !k.is_empty())
            .collect()
    }

    /// The dropdown's index for an action's name.
    fn index(&self, action: &str) -> usize {
        match action {
            "" => 0,
            a => (self.cmds.iter())
                .position(|(_, c)| name(*c) == a)
                .map_or(0, |i| i + 1),
        }
    }
}

pub fn view<'a>(
    k: &'a KeyEdit,
    cancel: Element<'a, Message>,
    font: crate::dialogs::ListFont,
) -> Element<'a, Message> {
    let msg = Message::KeyEdit;
    let mut list = column![].spacing(1);
    for (i, (key, act)) in k.list.iter().enumerate() {
        let waiting = k.capturing && k.sel == Some(i);
        let key = match (waiting, key.is_empty()) {
            (true, _) | (_, true) => fl!("keys-press"),
            _ => key.clone(),
        };
        list = list.push(crate::dialogs::menu_row(
            key,
            k.labels[k.index(act)].clone(),
            k.sel == Some(i),
            msg(KeyMsg::Select(i)),
            font,
        ));
    }
    if k.list.is_empty() {
        list = list.push(widget::text::body(fl!("keys-none")));
    }
    let list = widget::container(widget::scrollable(list).height(Length::Fixed(220.0)))
        .padding(4)
        .width(Length::Fill)
        .class(theme::Container::Card);
    let sel = k.sel.filter(|&i| i < k.list.len());
    let btn = |label: String, m: KeyMsg, on: bool| {
        widget::button::standard(label)
            .width(Length::Fixed(190.0))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let buttons = column![
        btn(fl!("keys-add"), KeyMsg::Add, true),
        btn(fl!("keys-rebind"), KeyMsg::Rebind, sel.is_some()),
        btn(fl!("hotlist-delete"), KeyMsg::Delete, sel.is_some()),
        btn(fl!("keys-reset"), KeyMsg::Reset, !k.list.is_empty()),
    ]
    .spacing(6);
    let picked = sel.map(|i| k.index(&k.list[i].1));
    let command = row![
        widget::text::body(fl!("keys-command")),
        widget::dropdown(&k.labels, picked, move |i| msg(KeyMsg::Command(i))).width(Length::Fill),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    widget::dialog()
        .title(fl!("keys-title"))
        .control(
            column![
                row![list, buttons].spacing(12),
                command,
                widget::text::caption(fl!("keys-hint")),
            ]
            .spacing(10)
            .width(Length::Fill),
        )
        .primary_action(widget::button::suggested(fl!("ok")).on_press(Message::DialogSubmit))
        .secondary_action(cancel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_once_and_names_resolve_back() {
        let cmds = commands();
        assert!(cmds.iter().any(|(_, a)| *a == Action::Copy));
        assert!(cmds.iter().any(|(_, a)| *a == Action::Enter)); // F1 only, not a menu item
        for (i, (_, a)) in cmds.iter().enumerate() {
            assert!(!cmds[..i].iter().any(|(_, b)| b == a), "{a:?} twice");
        }
        let own = resolve(&[
            ("ctrl+k".into(), "Copy".into()),
            ("F8".into(), String::new()),
            ("Ctrl+J".into(), "NoSuchThing".into()),
            ("Hyper+X".into(), "Copy".into()),
        ]);
        let k = Bind::parse("Ctrl+K").unwrap();
        assert_eq!(
            own,
            [(k, Some(Action::Copy)), (Bind::parse("F8").unwrap(), None)]
        );
    }

    #[test]
    fn add_capture_pick_and_one_row_per_key() {
        let mut k = KeyEdit::new(vec![("F8".into(), String::new())]);
        k.update(KeyMsg::Captured("Ctrl+K".into())); // not waiting: ignored
        assert_eq!(k.list[0].0, "F8");
        k.update(KeyMsg::Add);
        assert!(k.capturing);
        k.update(KeyMsg::Captured("Ctrl+K".into()));
        let copy = 1 + k.cmds.iter().position(|(_, a)| *a == Action::Copy).unwrap();
        k.update(KeyMsg::Command(copy));
        assert_eq!(k.list[1], ("Ctrl+K".into(), "Copy".into()));
        // The first row takes Ctrl+K too: the second goes.
        k.update(KeyMsg::Select(0));
        k.update(KeyMsg::Rebind);
        k.update(KeyMsg::Captured("Ctrl+K".into()));
        assert_eq!(k.list, [("Ctrl+K".to_string(), String::new())]);
        assert_eq!(k.sel, Some(0));
        k.update(KeyMsg::Add); // left without a key: not kept
        assert_eq!(k.clone().result().len(), 1);
        k.update(KeyMsg::Reset);
        assert!(k.result().is_empty());
    }
}
