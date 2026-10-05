//! TC's button bar: the buttons come from `config.toolbar`; right click → TC "Change button bar".
//! A button runs an internal command (`cm_*`, TC's names) or a program with `%P`-style parameters.

use crate::app::Message;
use crate::config::ToolButton;
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Length};
use cosmic::{Element, theme, widget};

/// A separator: a gap on the bar.
pub const SEP: &str = "-";

pub struct Builtin {
    pub cm: &'static str,
    pub action: Action,
    pub icon: &'static str,
    pub label: fn() -> String,
}

/// The internal commands a button can run; the first ones (up to the separators in
/// `default_bar`) make the default bar.
pub fn builtins() -> &'static [Builtin] {
    macro_rules! b {
        ($cm:literal, $a:ident, $icon:literal, $key:literal) => {
            Builtin {
                cm: $cm,
                action: Action::$a,
                icon: $icon,
                label: || fl!($key),
            }
        };
    }
    const LIST: &[Builtin] = &[
        b!(
            "cm_RereadSource",
            Reload,
            "view-refresh-symbolic",
            "menu-reload"
        ),
        b!("cm_SrcLong", ViewFull, "view-list-symbolic", "menu-full"),
        b!("cm_SrcShort", ViewBrief, "view-grid-symbolic", "menu-brief"),
        b!(
            "cm_SwitchHidSys",
            ToggleHidden,
            "view-reveal-symbolic",
            "menu-hidden"
        ),
        b!(
            "cm_Exchange",
            SwapPanes,
            "object-flip-horizontal-symbolic",
            "menu-swap"
        ),
        b!("cm_MkDir", Mkdir, "folder-new-symbolic", "menu-mkdir"),
        b!(
            "cm_SearchFor",
            FindFiles,
            "system-search-symbolic",
            "menu-find"
        ),
        b!(
            "cm_CompareDirs",
            CompareLists,
            "view-dual-symbolic",
            "menu-compare-lists"
        ),
        b!(
            "cm_SyncDirs",
            SyncDirs,
            "emblem-synchronizing-symbolic",
            "menu-sync"
        ),
        b!(
            "cm_PackFiles",
            Pack,
            "package-x-generic-symbolic",
            "menu-pack"
        ),
        b!(
            "cm_UnpackFiles",
            Unpack,
            "document-open-symbolic",
            "menu-unpack"
        ),
        b!(
            "cm_NetConnect",
            Connect,
            "network-server-symbolic",
            "menu-connect"
        ),
        b!(
            "cm_NetDisconnect",
            Disconnect,
            "media-eject-symbolic",
            "menu-disconnect"
        ),
        b!("cm_DirHotlist", Hotlist, "starred-symbolic", "menu-hotlist"),
        b!(
            "cm_Config",
            Settings,
            "emblem-system-symbolic",
            "menu-settings"
        ),
        b!("cm_List", View, "view-paged-symbolic", "menu-view"),
        b!("cm_Edit", Edit, "document-edit-symbolic", "menu-edit"),
        b!(
            "cm_EditNewFile",
            NewFile,
            "document-new-symbolic",
            "menu-new-file"
        ),
        b!("cm_Copy", Copy, "edit-copy-symbolic", "menu-copy"),
        b!("cm_RenMov", Move, "edit-cut-symbolic", "menu-move"),
        b!("cm_Delete", Delete, "edit-delete-symbolic", "menu-delete"),
        b!(
            "cm_MultiRenameFiles",
            MultiRename,
            "edit-find-replace-symbolic",
            "menu-multi-rename"
        ),
        b!(
            "cm_CompareFilesByContent",
            CompareFiles,
            "text-x-generic-symbolic",
            "menu-compare-files"
        ),
        b!(
            "cm_SelectAll",
            SelectAll,
            "edit-select-all-symbolic",
            "menu-select-all"
        ),
        b!("cm_OpenNewTab", NewTab, "tab-new-symbolic", "menu-new-tab"),
        b!(
            "cm_SetAttrib",
            Properties,
            "document-properties-symbolic",
            "menu-properties"
        ),
        b!(
            "cm_CountDirContent",
            CountDirs,
            "drive-harddisk-symbolic",
            "menu-count-dirs"
        ),
        b!(
            "cm_ConfigToolbars",
            ConfigureToolbar,
            "applications-system-symbolic",
            "menu-toolbar"
        ),
    ];
    LIST
}

pub fn builtin(cmd: &str) -> Option<&'static Builtin> {
    builtins().iter().find(|b| b.cm == cmd)
}

pub fn default_bar() -> Vec<ToolButton> {
    let cm = |b: &Builtin| ToolButton {
        cmd: b.cm.into(),
        ..Default::default()
    };
    let sep = || ToolButton {
        cmd: SEP.into(),
        ..Default::default()
    };
    let l = builtins();
    [&l[..4], &l[4..9], &l[9..11], &l[11..15]]
        .iter()
        .enumerate()
        .flat_map(|(i, g)| (i > 0).then(sep).into_iter().chain(g.iter().map(cm)))
        .collect()
}

/// Icon name: the button's own, else its command's.
fn icon(b: &ToolButton) -> &str {
    match (b.icon.as_str(), builtin(&b.cmd)) {
        ("", Some(c)) => c.icon,
        ("", None) => "application-x-executable-symbolic",
        (i, _) => i,
    }
}

/// Tooltip: the button's own, else its command's name.
fn tip(b: &ToolButton) -> String {
    match (b.tip.as_str(), builtin(&b.cmd)) {
        ("", Some(c)) => (c.label)(),
        ("", None) => b.cmd.clone(),
        (t, _) => t.into(),
    }
}

pub fn bar(buttons: &[ToolButton]) -> Element<'static, Message> {
    use cosmic::widget::tooltip::{Position, tooltip};
    let items = buttons
        .iter()
        .enumerate()
        .map(|(i, b)| -> Element<'static, Message> {
            if b.cmd == SEP {
                return widget::Space::new().width(Length::Fixed(10.0)).into();
            }
            tooltip(
                widget::button::icon(widget::icon::from_name(icon(b)).size(16))
                    .on_press(Message::Tool(i)),
                widget::text(tip(b)).size(13),
                Position::Bottom,
            )
            .into()
        });
    let row = widget::row::with_children(items.collect::<Vec<_>>())
        .spacing(2)
        .padding([0, 6, 2, 6])
        .height(Length::Fixed(34.0))
        .width(Length::Fill)
        .align_y(Alignment::Center);
    widget::mouse_area(row)
        .on_right_press(Message::Key(Action::ConfigureToolbar))
        .into()
}

#[derive(Clone, Debug)]
pub struct ToolEdit {
    pub list: Vec<ToolButton>,
    pub sel: Option<usize>,
    /// Dropdown labels, in `builtins()` order.
    labels: Vec<String>,
    /// The icon picker shown instead of the editor: (all names, filter).
    pub picker: Option<(Vec<String>, String)>,
}

/// The picker draws at most this many icons; the filter narrows the rest down.
const PICKER_MAX: usize = 240;
const PICKER_COLS: usize = 12;

/// Symbolic icons of the current theme and its usual fallbacks, from the XDG icon dirs. Slow
/// (seconds: every name goes through the icon lookup), so run off the UI thread, once.
// ponytail: kept for the process; a theme switch needs a restart to show its own set.
pub fn icon_names() -> Vec<String> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(scan_icons).clone()
}

fn scan_icons() -> Vec<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let roots: Vec<_> = (home.map(|h| h.join(".icons")).into_iter())
        .chain(data_home.map(|d| d.join("icons")))
        .chain(
            data_dirs
                .split(':')
                .map(|d| std::path::Path::new(d).join("icons")),
        )
        .collect();
    let theme = cosmic::icon_theme::default();
    let mut names = shagoff_core::icons::symbolic(&roots, &[&theme, "Adwaita", "hicolor"]);
    // Only what the icon loader finds (a file of a theme outside its lookup chain draws blank).
    names.retain(|n| widget::icon::from_name(n.as_str()).path().is_some());
    names
}

#[derive(Clone, Debug)]
pub enum ToolMsg {
    Select(usize),
    /// The internal command `builtins()[i]` (its icon too, no parameters).
    Builtin(usize),
    Cmd(String),
    Params(String),
    Icon(String),
    Tip(String),
    Add,
    AddSep,
    Delete,
    Up,
    Down,
    /// Back to the default bar.
    Reset,
    /// Open the icon picker / its filter / pick one (back to the editor) / back without one.
    PickOpen,
    /// `icon_names()`, loaded in the background after `PickOpen`.
    PickLoaded(Vec<String>),
    PickFilter(String),
    Pick(String),
    PickClose,
}

impl ToolEdit {
    pub fn new(list: Vec<ToolButton>) -> Self {
        let sel = (!list.is_empty()).then_some(0);
        Self {
            list,
            sel,
            labels: builtins().iter().map(|b| (b.label)()).collect(),
            picker: None,
        }
    }

    fn insert(&mut self, b: ToolButton) {
        let at = self.sel.map_or(self.list.len(), |i| i + 1);
        self.list.insert(at, b);
        self.sel = Some(at);
    }

    pub fn update(&mut self, m: ToolMsg) {
        let sel = self.sel.filter(|&i| i < self.list.len());
        let editable = sel.filter(|&i| self.list[i].cmd != SEP);
        match (m, editable) {
            (ToolMsg::Select(i), _) => self.sel = (i < self.list.len()).then_some(i),
            (ToolMsg::Builtin(b), Some(i)) => {
                if let Some(c) = builtins().get(b) {
                    self.list[i] = ToolButton {
                        cmd: c.cm.into(),
                        ..Default::default()
                    };
                }
            }
            (ToolMsg::Cmd(s), Some(i)) if s != SEP => self.list[i].cmd = s,
            (ToolMsg::Params(s), Some(i)) => self.list[i].params = s,
            (ToolMsg::Icon(s), Some(i)) => self.list[i].icon = s,
            (ToolMsg::Tip(s), Some(i)) => self.list[i].tip = s,
            (ToolMsg::Add, _) => self.insert(ToolButton {
                icon: "utilities-terminal-symbolic".into(),
                ..Default::default()
            }),
            (ToolMsg::AddSep, _) => self.insert(ToolButton {
                cmd: SEP.into(),
                ..Default::default()
            }),
            (ToolMsg::Delete, _) => {
                if let Some(i) = sel {
                    self.list.remove(i);
                    self.sel = (!self.list.is_empty()).then(|| i.min(self.list.len() - 1));
                }
            }
            (ToolMsg::Up, _) => {
                if let Some(i) = sel.filter(|&i| i > 0) {
                    self.list.swap(i, i - 1);
                    self.sel = Some(i - 1);
                }
            }
            (ToolMsg::Down, _) => {
                if let Some(i) = sel.filter(|&i| i + 1 < self.list.len()) {
                    self.list.swap(i, i + 1);
                    self.sel = Some(i + 1);
                }
            }
            (ToolMsg::Reset, _) => *self = Self::new(default_bar()),
            (ToolMsg::PickOpen, Some(_)) => self.picker = Some((Vec::new(), String::new())),
            (ToolMsg::PickLoaded(names), _) => {
                if let Some((all, _)) = &mut self.picker {
                    *all = names;
                }
            }
            (ToolMsg::PickFilter(s), _) => {
                if let Some((_, f)) = &mut self.picker {
                    *f = s;
                }
            }
            (ToolMsg::Pick(name), Some(i)) => {
                self.list[i].icon = name;
                self.picker = None;
            }
            (ToolMsg::PickClose, _) => self.picker = None,
            _ => {}
        }
    }
}

/// The names matching `filter` (each word, any case), at most `PICKER_MAX`, and how many matched.
fn matching<'a>(all: &'a [String], filter: &str) -> (Vec<&'a str>, usize) {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let hits: Vec<&str> = (all.iter())
        .filter(|n| words.iter().all(|w| n.contains(w.as_str())))
        .map(String::as_str)
        .collect();
    let n = hits.len();
    (hits.into_iter().take(PICKER_MAX).collect(), n)
}

fn picker_view<'a>(all: &'a [String], filter: &'a str) -> Element<'a, Message> {
    use cosmic::widget::tooltip::{Position, tooltip};
    let msg = Message::ToolEdit;
    let (shown, total) = matching(all, filter);
    let cell = |name: &'a str| -> Element<'a, Message> {
        tooltip(
            widget::button::icon(widget::icon::from_name(name).size(24))
                .on_press(msg(ToolMsg::Pick(name.into()))),
            widget::text(name).size(12),
            Position::Top,
        )
        .into()
    };
    // Rows by hand: flex_row in a scrollable lays everything out on one line.
    let grid = column(
        shown
            .chunks(PICKER_COLS)
            .map(|r| row(r.iter().map(|&n| cell(n))).spacing(4).into()),
    )
    .spacing(4);
    widget::dialog()
        .title(fl!("toolbar-pick-title"))
        .control(
            column![
                widget::text_input(fl!("toolbar-pick-filter"), filter)
                    .on_input(move |s| msg(ToolMsg::PickFilter(s))),
                widget::container(widget::scrollable(grid).height(Length::Fixed(250.0)))
                    .padding(6)
                    .width(Length::Fill)
                    .class(theme::Container::Card),
                widget::text::caption(if all.is_empty() {
                    fl!("toolbar-pick-loading")
                } else {
                    fl!("toolbar-pick-count", shown = shown.len(), total = total)
                }),
            ]
            .spacing(8)
            .width(Length::Fill),
        )
        .secondary_action(
            widget::button::standard(fl!("toolbar-pick-back")).on_press(msg(ToolMsg::PickClose)),
        )
        .into()
}

pub fn view<'a>(t: &'a ToolEdit, cancel: Element<'a, Message>) -> Element<'a, Message> {
    let msg = Message::ToolEdit;
    if let Some((all, filter)) = &t.picker {
        return picker_view(all, filter);
    }
    let mut list = column![].spacing(1);
    for (i, b) in t.list.iter().enumerate() {
        let on = msg(ToolMsg::Select(i));
        list = list.push(if b.cmd == SEP {
            crate::hotlist::sep_row(t.sel == Some(i), on)
        } else {
            let line = row![
                widget::icon::from_name(icon(b)).size(16),
                crate::dialogs::menu_row(tip(b), b.cmd.clone(), t.sel == Some(i), on),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            line.into()
        });
    }
    let list = widget::container(widget::scrollable(list).height(Length::Fixed(150.0)))
        .padding(4)
        .width(Length::Fill)
        .class(theme::Container::Card);

    let sel = t.sel.filter(|&i| i < t.list.len());
    let btn = |label: String, m: ToolMsg, on: bool| {
        widget::button::standard(label)
            .width(Length::Fixed(170.0))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let arrow = |name: &'static str, m: ToolMsg, on: bool| {
        widget::button::icon(widget::icon::from_name(name).size(16))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let buttons = column![
        btn(fl!("toolbar-add"), ToolMsg::Add, true),
        btn(fl!("hotlist-add-sep"), ToolMsg::AddSep, true),
        btn(fl!("hotlist-delete"), ToolMsg::Delete, sel.is_some()),
        btn(fl!("toolbar-reset"), ToolMsg::Reset, true),
        row![
            arrow("go-up-symbolic", ToolMsg::Up, sel.is_some_and(|i| i > 0)),
            arrow(
                "go-down-symbolic",
                ToolMsg::Down,
                sel.is_some_and(|i| i + 1 < t.list.len())
            ),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let editable = sel.map(|i| &t.list[i]).filter(|b| b.cmd != SEP);
    let field = |label: String, hint: String, value: &'a str, on: Option<fn(String) -> ToolMsg>| {
        let mut input = widget::text_input(hint, value);
        if let Some(f) = on {
            input = input
                .on_input(move |s| msg(f(s)))
                .on_submit(|_| Message::DialogSubmit);
        }
        row![widget::text::body(label).width(Length::Fixed(90.0)), input]
            .spacing(6)
            .align_y(Alignment::Center)
    };
    let on = |f: fn(String) -> ToolMsg| editable.map(|_| f);
    let b = editable.cloned().unwrap_or_default();
    let internal = editable.and_then(|b| builtins().iter().position(|c| c.cm == b.cmd));
    let (cmd, params, icon_name, tip_text) = match editable {
        Some(b) => (&*b.cmd, &*b.params, &*b.icon, &*b.tip),
        None => ("", "", "", ""),
    };
    let fields = column![
        row![
            field(
                fl!("toolbar-command"),
                fl!("toolbar-command-hint"),
                cmd,
                on(ToolMsg::Cmd)
            ),
            widget::dropdown(&t.labels, internal, move |i| msg(ToolMsg::Builtin(i)))
                .width(Length::Fixed(200.0)),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        field(
            fl!("toolbar-params"),
            fl!("toolbar-hint"),
            params,
            on(ToolMsg::Params).filter(|_| internal.is_none())
        ),
        row![
            field(
                fl!("toolbar-icon"),
                icon(&b).to_string(),
                icon_name,
                on(ToolMsg::Icon)
            ),
            widget::button::standard(fl!("toolbar-pick"))
                .leading_icon(widget::icon::from_name(icon(&b)).size(16))
                .on_press_maybe(editable.map(|_| msg(ToolMsg::PickOpen))),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        field(fl!("toolbar-tip"), tip(&b), tip_text, on(ToolMsg::Tip)),
    ]
    .spacing(6);

    widget::dialog()
        .title(fl!("toolbar-title"))
        .control(
            column![row![list, buttons].spacing(12), fields]
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

    fn cmds(t: &ToolEdit) -> Vec<&str> {
        t.list.iter().map(|b| b.cmd.as_str()).collect()
    }

    #[test]
    fn picker_filters_by_every_word_and_picks() {
        let all: Vec<String> = ["edit-copy-symbolic", "edit-cut-symbolic", "go-up-symbolic"]
            .map(String::from)
            .to_vec();
        assert_eq!(matching(&all, "  Copy EDIT ").0, ["edit-copy-symbolic"]);
        assert_eq!(matching(&all, "").1, 3);
        let mut t = ToolEdit::new(default_bar());
        t.picker = Some((all, String::new()));
        t.update(ToolMsg::Pick("go-up-symbolic".into()));
        assert_eq!(
            (t.list[0].icon.as_str(), t.picker.is_none()),
            ("go-up-symbolic", true)
        );
    }

    #[test]
    fn default_bar_has_its_groups_and_known_commands() {
        let bar = default_bar();
        assert_eq!(bar.iter().filter(|b| b.cmd == SEP).count(), 3);
        assert!(
            bar.iter()
                .all(|b| b.cmd == SEP || builtin(&b.cmd).is_some())
        );
        let mut seen = std::collections::HashSet::new();
        assert!(
            builtins().iter().all(|b| seen.insert(b.cm)),
            "cm names are unique"
        );
    }

    #[test]
    fn edit_add_move_delete_reset() {
        let mut t = ToolEdit::new(vec![]);
        t.update(ToolMsg::Add);
        t.update(ToolMsg::Cmd("xterm".into()));
        t.update(ToolMsg::Params("-e %P".into()));
        t.update(ToolMsg::AddSep);
        t.update(ToolMsg::Cmd("x".into())); // a separator has no command
        t.update(ToolMsg::Up);
        assert_eq!(cmds(&t), ["-", "xterm"]);
        assert_eq!(t.list[1].params, "-e %P");
        t.update(ToolMsg::Select(1));
        t.update(ToolMsg::Builtin(0)); // internal: params and icon dropped
        assert_eq!(
            (t.list[1].cmd.as_str(), t.list[1].params.as_str()),
            ("cm_RereadSource", "")
        );
        t.update(ToolMsg::Down); // last: stays
        t.update(ToolMsg::Delete);
        t.update(ToolMsg::Delete);
        assert_eq!((cmds(&t).len(), t.sel), (0, None));
        t.update(ToolMsg::Reset);
        assert_eq!((t.list.clone(), t.sel), (default_bar(), Some(0)));
    }
}
