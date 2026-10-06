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

/// A dir's name as a favourite's name: its `&` doubled, so it is no hot letter (`R&D`).
pub fn menu_name(dir: &std::path::Path) -> String {
    format::dir_title(dir).replace('&', "&&")
}

pub fn is_sep(e: &HotEntry) -> bool {
    e.name == SEP && e.path.as_os_str().is_empty()
}

/// TC's submenu: `-Name` with no path starts one, `--` ends it.
pub const END: &str = "--";

pub fn is_sub(e: &HotEntry) -> bool {
    e.name.starts_with('-') && e.name != SEP && e.name != END && e.path.as_os_str().is_empty()
}

pub fn is_end(e: &HotEntry) -> bool {
    e.name == END && e.path.as_os_str().is_empty()
}

/// A submenu's caption (its name without the leading `-`).
pub fn sub_name(e: &HotEntry) -> &str {
    e.name.strip_prefix('-').unwrap_or(&e.name)
}

/// Indices of the entries directly in the submenu starting at `start` (the top level for
/// `None`): nested submenus count as one entry, their contents are skipped. An `--` with
/// nothing open is ignored; a submenu with no `--` runs to the end.
pub fn children(list: &[HotEntry], start: Option<usize>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for (i, e) in list.iter().enumerate().skip(start.map_or(0, |s| s + 1)) {
        if is_end(e) {
            match depth {
                0 if start.is_some() => break,
                0 => {}
                _ => depth -= 1,
            }
        } else {
            if depth == 0 {
                out.push(i);
            }
            if is_sub(e) {
                depth += 1;
            }
        }
    }
    out
}

/// A line or a submenu bound: no path, not a dir.
pub fn is_marker(e: &HotEntry) -> bool {
    is_sep(e) || is_sub(e) || is_end(e)
}

/// Where the submenu starting at `start` ends: its `--`, or the end of the list.
pub fn sub_end(list: &[HotEntry], start: usize) -> usize {
    let mut depth = 0usize;
    for (i, e) in list.iter().enumerate().skip(start + 1) {
        if is_end(e) {
            if depth == 0 {
                return i;
            }
            depth -= 1;
        } else if is_sub(e) {
            depth += 1;
        }
    }
    list.len()
}

/// Nesting level of each entry, for indenting the editor's list (an `--` sits at its submenu's).
pub fn depths(list: &[HotEntry]) -> Vec<usize> {
    let mut depth = 0usize;
    list.iter()
        .map(|e| {
            if is_end(e) {
                depth = depth.saturating_sub(1);
                return depth;
            }
            let d = depth;
            if is_sub(e) {
                depth += 1;
            }
            d
        })
        .collect()
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
    /// A submenu: `-Name` and its `--` after the selection, the name selected.
    AddSub,
    Delete,
    Up,
    Down,
    /// Alphabetically, each block between separators and submenu bounds on its own (as TC).
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
            // A submenu keeps its `-`; an empty name would turn it into a separator.
            (HotMsg::Name(s), Some(i)) if is_sub(&self.list[i]) => {
                if !s.is_empty() {
                    self.list[i].name = format!("-{s}");
                }
            }
            (HotMsg::Name(s), Some(i)) if !is_sep(&self.list[i]) && !is_end(&self.list[i]) => {
                self.list[i].name = s
            }
            (HotMsg::Path(s), Some(i)) if !is_marker(&self.list[i]) => {
                self.list[i].path = s.clone().into();
                self.path = s;
            }
            (HotMsg::Here, Some(i)) if !is_marker(&self.list[i]) => {
                self.list[i].path = cwd.into();
                self.select(i);
            }
            (HotMsg::Add, _) => self.insert(HotEntry {
                name: menu_name(cwd),
                path: cwd.into(),
            }),
            (HotMsg::AddSep, _) => self.insert(HotEntry {
                name: SEP.into(),
                path: Default::default(),
            }),
            (HotMsg::AddSub, _) => {
                let mark = |name: String| HotEntry {
                    name,
                    path: Default::default(),
                };
                self.insert(mark(END.into()));
                self.list.insert(
                    self.sel.unwrap_or(0),
                    mark(format!("-{}", fl!("hotlist-new-sub"))),
                );
                // The `--` moves one down; the selection now is the submenu's name.
            }
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
                for block in self.list.split_mut(is_marker) {
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
    for ((i, e), depth) in h.list.iter().enumerate().zip(depths(&h.list)) {
        let (on, sel) = (msg(HotMsg::Select(i)), h.sel == Some(i));
        let row: Element<'a, Message> = match () {
            _ if is_sep(e) => sep_row(sel, on),
            _ if is_sub(e) => {
                crate::dialogs::menu_row(format!("▸ {}", sub_name(e)), String::new(), sel, on)
            }
            _ if is_end(e) => {
                crate::dialogs::menu_row(fl!("hotlist-sub-end"), String::new(), sel, on)
            }
            _ => crate::dialogs::menu_row(e.name.clone(), e.path.display().to_string(), sel, on),
        };
        // Submenu contents indented, as the menu nests them.
        list = list.push(widget::container(row).padding([0, 0, 0, 16 * depth as u16]));
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
    let arrow = |name: &'static str, m: HotMsg, on: bool| {
        widget::button::icon(widget::icon::from_name(name).size(16))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let last = h.sel.is_some_and(|i| i + 1 >= h.list.len());
    let buttons = column![
        btn(fl!("hotlist-add-item"), HotMsg::Add, true),
        btn(fl!("hotlist-add-sep"), HotMsg::AddSep, true),
        btn(fl!("hotlist-add-sub"), HotMsg::AddSub, true),
        btn(fl!("hotlist-delete"), HotMsg::Delete, some),
        btn(fl!("hotlist-sort"), HotMsg::Sort, h.list.len() > 1),
        row![
            arrow("go-up-symbolic", HotMsg::Up, h.sel.is_some_and(|i| i > 0)),
            arrow("go-down-symbolic", HotMsg::Down, some && !last),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let selected = h.sel.and_then(|i| h.list.get(i));
    // A submenu has a name but no path; a line and an `--` have neither.
    let named = selected.filter(|e| !is_sep(e) && !is_end(e));
    let editable = named.filter(|e| !is_sub(e));
    let name = named.map_or("", |e| if is_sub(e) { sub_name(e) } else { &e.name });
    let mut name_in = widget::text_input("", name);
    let mut path_in = widget::text_input("", &h.path);
    if named.is_some() {
        name_in = name_in
            .on_input(move |s| msg(HotMsg::Name(s)))
            .on_submit(|_| Message::DialogSubmit);
    }
    if editable.is_some() {
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
    fn add_sub_rename_it_and_sort_inside() {
        let cwd = Path::new("/c");
        let mut h = HotEdit::new(0, vec![e("z"), e("a")]);
        h.update(HotMsg::AddSub, cwd); // after "z": its name selected
        assert!(is_sub(&h.list[1]) && is_end(&h.list[2]) && h.sel == Some(1));
        h.update(HotMsg::Name("Work".into()), cwd);
        h.update(HotMsg::Name(String::new()), cwd); // never a separator
        h.update(HotMsg::Path("/x".into()), cwd); // a submenu has no path
        assert_eq!(
            (
                h.list[1].name.as_str(),
                h.list[1].path.as_os_str().is_empty()
            ),
            ("-Work", true)
        );
        h.update(HotMsg::Select(2), cwd);
        h.update(HotMsg::Name("x".into()), cwd); // `--` keeps its name
        h.list.insert(2, e("y"));
        h.list.insert(2, e("b"));
        h.list.insert(2, e("y2"));
        h.update(HotMsg::Sort, cwd); // inside the submenu, around it apart
        assert_eq!(names(&h), ["z", "-Work", "b", "y", "y2", "--", "a"]);
        assert_eq!(children(&h.list, Some(1)), [2, 3, 4]);
    }

    fn m(n: &str) -> HotEntry {
        HotEntry {
            name: n.into(),
            path: Default::default(),
        }
    }

    #[test]
    fn submenus_nest_and_skip_their_contents() {
        // a, [Work: b, [Deep: c], d], -, e, stray --
        let list = [
            e("a"),
            m("-Work"),
            e("b"),
            m("-Deep"),
            e("c"),
            m("--"),
            e("d"),
            m("--"),
            m("-"),
            e("e"),
            m("--"),
        ];
        assert!(is_sub(&list[1]) && !is_sub(&list[8]) && !is_sub(&list[5]) && !is_sub(&e("-x")));
        assert_eq!(sub_name(&list[1]), "Work");
        assert_eq!(children(&list, None), [0, 1, 8, 9]);
        assert_eq!(children(&list, Some(1)), [2, 3, 6]);
        assert_eq!(children(&list, Some(3)), [4]);
        assert_eq!(depths(&list), [0, 0, 1, 1, 2, 1, 1, 0, 0, 0, 0]);
        assert_eq!((sub_end(&list, 1), sub_end(&list, 3)), (7, 5));
        let open = [m("-Open"), e("x")]; // no end: runs to the end
        assert_eq!(children(&open, Some(0)), [1]);
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
