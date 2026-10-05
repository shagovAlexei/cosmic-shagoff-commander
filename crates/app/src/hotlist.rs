//! Ctrl+D → "Configure…": TC's directory hotlist settings. Edits a copy, saved on OK.

use crate::app::Message;
use crate::config::HotEntry;
use crate::fl;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Length};
use cosmic::{Element, theme, widget};
use shagoff_core::format;
use std::path::Path;

/// TC's menu separator: an entry named `-` with no path.
pub const SEP: &str = "-";

pub fn is_sep(e: &HotEntry) -> bool {
    e.name == SEP && e.path.as_os_str().is_empty()
}

#[derive(Clone, Debug)]
pub struct HotEdit {
    pub side: usize,
    pub list: Vec<HotEntry>,
    pub sel: Option<usize>,
    /// The selected entry's path as typed (a `PathBuf` can't back a text field).
    pub path: String,
}

#[derive(Clone, Debug)]
pub enum HotMsg {
    Select(usize),
    Name(String),
    Path(String),
    /// TC "Add item": the panel's dir, after the selection.
    Add,
    AddSep,
    Delete,
    Up,
    Down,
    /// Alphabetically, each block between separators on its own (TC does the same).
    Sort,
    /// The selected entry gets the panel's dir.
    Here,
}

impl HotEdit {
    pub fn new(side: usize, list: Vec<HotEntry>) -> Self {
        let mut h = Self {
            side,
            list,
            sel: None,
            path: String::new(),
        };
        if !h.list.is_empty() {
            h.select(0);
        }
        h
    }

    fn select(&mut self, i: usize) {
        self.sel = self.list.get(i).map(|_| i);
        self.path = self
            .list
            .get(i)
            .map_or_else(String::new, |e| e.path.display().to_string());
    }

    fn insert(&mut self, e: HotEntry) {
        let at = self.sel.map_or(self.list.len(), |i| i + 1);
        self.list.insert(at, e);
        self.select(at);
    }

    /// `cwd`: the panel's dir, for Add and Here.
    pub fn update(&mut self, m: HotMsg, cwd: &Path) {
        let sel = self.sel.filter(|&i| i < self.list.len());
        match (m, sel) {
            (HotMsg::Select(i), _) => self.select(i),
            (HotMsg::Name(s), Some(i)) => self.list[i].name = s,
            (HotMsg::Path(s), Some(i)) if !is_sep(&self.list[i]) => {
                self.list[i].path = s.clone().into();
                self.path = s;
            }
            (HotMsg::Here, Some(i)) if !is_sep(&self.list[i]) => {
                self.list[i].path = cwd.into();
                self.select(i);
            }
            (HotMsg::Add, _) => self.insert(HotEntry {
                name: format::dir_title(cwd),
                path: cwd.into(),
            }),
            (HotMsg::AddSep, _) => self.insert(HotEntry {
                name: SEP.into(),
                path: Default::default(),
            }),
            (HotMsg::Delete, Some(i)) => {
                self.list.remove(i);
                self.select(i.min(self.list.len().saturating_sub(1)));
            }
            (HotMsg::Up, Some(i)) if i > 0 => {
                self.list.swap(i, i - 1);
                self.select(i - 1);
            }
            (HotMsg::Down, Some(i)) if i + 1 < self.list.len() => {
                self.list.swap(i, i + 1);
                self.select(i + 1);
            }
            (HotMsg::Sort, _) => {
                let kept = sel.map(|i| self.list[i].clone());
                for block in self.list.split_mut(is_sep) {
                    block.sort_by_key(|e| e.name.to_lowercase());
                }
                if let Some(k) = kept {
                    let i = self.list.iter().position(|e| *e == k).unwrap_or(0);
                    self.select(i);
                }
            }
            _ => {}
        }
    }
}

/// A separator in a list dialog: selectable (to move or delete it), drawn as a line.
pub fn sep_row<'a>(selected: bool, on: Message) -> Element<'a, Message> {
    widget::button::custom(
        widget::container(widget::divider::horizontal::default())
            .padding([0, 8])
            .width(Length::Fill)
            .height(Length::Fixed(14.0))
            .align_y(Alignment::Center)
            .class(crate::view::cursor_style(
                selected,
                true,
                false,
                crate::config::Skin::Modern,
            )),
    )
    .padding(0)
    .width(Length::Fill)
    .class(theme::Button::MenuItem)
    .on_press(on)
    .into()
}

pub fn view<'a>(h: &'a HotEdit, cancel: Element<'a, Message>) -> Element<'a, Message> {
    let msg = |m: HotMsg| Message::Hot(m);
    let mut list = column![].spacing(1);
    for (i, e) in h.list.iter().enumerate() {
        let row: Element<'a, Message> = if is_sep(e) {
            sep_row(h.sel == Some(i), msg(HotMsg::Select(i)))
        } else {
            crate::dialogs::menu_row(
                e.name.clone(),
                e.path.display().to_string(),
                h.sel == Some(i),
                msg(HotMsg::Select(i)),
            )
        };
        list = list.push(row);
    }
    if h.list.is_empty() {
        list = list.push(widget::text::caption(fl!("hotlist-empty")));
    }
    let list = widget::container(widget::scrollable(list).height(Length::Fixed(240.0)))
        .padding(4)
        .width(Length::Fill)
        .class(theme::Container::Card);

    let some = h.sel.is_some();
    let btn = |label: String, m: HotMsg, on: bool| {
        widget::button::standard(label)
            .width(Length::Fixed(150.0))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let last = h.sel.is_some_and(|i| i + 1 >= h.list.len());
    let buttons = column![
        btn(fl!("hotlist-add-item"), HotMsg::Add, true),
        btn(fl!("hotlist-add-sep"), HotMsg::AddSep, true),
        btn(fl!("hotlist-delete"), HotMsg::Delete, some),
        btn(fl!("hotlist-up"), HotMsg::Up, h.sel.is_some_and(|i| i > 0)),
        btn(fl!("hotlist-down"), HotMsg::Down, some && !last),
        btn(fl!("hotlist-sort"), HotMsg::Sort, h.list.len() > 1),
    ]
    .spacing(6);

    let editable = h.sel.and_then(|i| h.list.get(i)).filter(|e| !is_sep(e));
    let name = editable.map_or("", |e| e.name.as_str());
    let mut name_in = widget::text_input("", name);
    let mut path_in = widget::text_input("", &h.path);
    if editable.is_some() {
        name_in = name_in
            .on_input(move |s| msg(HotMsg::Name(s)))
            .on_submit(|_| Message::DialogSubmit);
        path_in = path_in
            .on_input(move |s| msg(HotMsg::Path(s)))
            .on_submit(|_| Message::DialogSubmit);
    }
    let fields = column![
        widget::text::caption(fl!("hotlist-name")),
        name_in,
        widget::text::caption(fl!("hotlist-path")),
        row![
            path_in,
            widget::button::standard(fl!("hotlist-here"))
                .on_press_maybe(editable.map(|_| msg(HotMsg::Here))),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(4);

    widget::dialog()
        .title(fl!("hotlist-configure-title"))
        .control(
            column![row![list, buttons].spacing(12), fields]
                .spacing(12)
                .width(Length::Fill),
        )
        .primary_action(widget::button::suggested(fl!("ok")).on_press(Message::DialogSubmit))
        .secondary_action(cancel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: &str) -> HotEntry {
        HotEntry {
            name: n.into(),
            path: format!("/{n}").into(),
        }
    }

    fn names(h: &HotEdit) -> Vec<&str> {
        h.list.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn add_sep_move_sort_delete() {
        let cwd = Path::new("/x/here");
        let mut h = HotEdit::new(0, vec![e("b"), e("a")]);
        assert_eq!(h.sel, Some(0));
        h.update(HotMsg::AddSep, cwd); // after "b"
        h.update(HotMsg::Add, cwd); // after the separator
        assert_eq!(names(&h), ["b", "-", "here", "a"]);
        assert_eq!((h.sel, h.path.as_str()), (Some(2), "/x/here"));
        h.update(HotMsg::Down, cwd);
        assert_eq!(names(&h), ["b", "-", "a", "here"]);
        h.update(HotMsg::Down, cwd); // last: stays
        assert_eq!(h.sel, Some(3));
        h.update(HotMsg::Sort, cwd); // blocks apart, selection follows its entry
        assert_eq!(names(&h), ["b", "-", "a", "here"]);
        h.update(HotMsg::Select(0), cwd);
        h.update(HotMsg::Up, cwd); // first: stays
        h.update(HotMsg::Delete, cwd);
        assert_eq!(names(&h), ["-", "a", "here"]);
        assert_eq!(h.sel, Some(0));
        h.update(HotMsg::Path("/y".into()), cwd); // a separator has no path
        assert!(h.list[0].path.as_os_str().is_empty());
    }

    #[test]
    fn edit_name_path_and_here() {
        let mut h = HotEdit::new(0, vec![e("a")]);
        h.update(HotMsg::Name("Docs".into()), Path::new("/c"));
        h.update(HotMsg::Path("/docs".into()), Path::new("/c"));
        assert_eq!(
            h.list[0],
            HotEntry {
                name: "Docs".into(),
                path: "/docs".into()
            }
        );
        h.update(HotMsg::Here, Path::new("/c"));
        assert_eq!(
            (h.list[0].path.to_str(), h.path.as_str()),
            (Some("/c"), "/c")
        );
        h.update(HotMsg::Delete, Path::new("/c"));
        assert_eq!((h.sel, h.path.as_str()), (None, ""));
    }
}
