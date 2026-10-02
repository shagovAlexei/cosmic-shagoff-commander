use crate::dialogs::{self, Dialog, InputOp};
use crate::fl;
use crate::keymap::{self, Action};
use cosmic::app::{Core, Task};
use cosmic::iced::keyboard::Modifiers;
use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
use cosmic::iced::{Subscription, event, keyboard};
use cosmic::{Application, Element, widget};
use shagoff_core::format::TimeZone;
use shagoff_core::listing::{self, Entry};
use shagoff_core::mask::Mask;
use shagoff_core::panel::{PARENT, Panel};
use shagoff_core::sort::SortKey;
use shagoff_core::tabs::Tabs;
use shagoff_core::viewport;
use std::path::{Component, Path, PathBuf};

pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";
/// Fixed row height of the file list; the viewport math depends on it.
pub const ROW_H: f32 = 22.0;
// ponytail: list height is guessed until the scrollable reports its bounds (it does on the first event).
const FALLBACK_LIST_H: f32 = 400.0;

pub struct Flags {
    pub left: Option<PathBuf>,
}

pub struct Tab {
    /// Stable id: scan results are routed by it, so they land in the right tab even after switching.
    pub id: u64,
    pub panel: Panel,
    pub offset: f32,
    pub height: f32,
    /// Path of the scan in flight; results for any other path are stale.
    pending: Option<PathBuf>,
    pub error: Option<String>,
}

impl Tab {
    fn new(id: u64, cwd: PathBuf) -> Self {
        Self {
            id,
            panel: Panel::new(cwd),
            offset: 0.0,
            height: FALLBACK_LIST_H,
            pending: None,
            error: None,
        }
    }

    /// Copy for Ctrl+T: same dir, sort, cursor and scroll; fresh id.
    fn duplicate(&self, id: u64) -> Self {
        Self {
            id,
            panel: {
                // TC: a new tab starts without a selection
                let mut panel = self.panel.clone();
                panel.mark_all(false);
                panel
            },
            offset: self.offset,
            height: self.height,
            pending: None,
            error: None,
        }
    }
}

pub struct App {
    core: Core,
    pub panes: [Tabs<Tab>; 2],
    /// One scroll widget id per pane, not per tab: iced's tree diff keeps the first id it saw at a
    /// position (`Tree::diff` → `set_id` for `Internal::Set` ids), so per-tab ids would never match.
    pub scroll_ids: [widget::Id; 2],
    pub active: usize,
    pub tz: TimeZone,
    next_id: u64,
    /// Current keyboard modifiers, for Ctrl+click.
    mods: Modifiers,
    dialog: Option<Dialog>,
    /// Id of the dialog text field (one field at a time), for focusing it on open.
    input_id: widget::Id,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key(Action),
    Listed {
        side: usize,
        tab: u64,
        path: PathBuf,
        focus: Option<String>,
        result: Result<Vec<Entry>, String>,
    },
    Click(usize, usize),
    DoubleClick(usize, usize),
    Header(usize, SortKey),
    /// side, scroll offset y, viewport height (of the active tab)
    Scrolled(usize, f32, f32),
    SelectTab(usize, usize),
    CloseTabAt(usize, usize),
    Modifiers(Modifiers),
    DialogInput(String),
    DialogSubmit,
    DialogCancel,
    Exit,
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, flags: Flags) -> (Self, Task<Message>) {
        core.window.header_title = fl!("app-title");
        // Tab is ours (switch pane); libcosmic's Tab focus-walk would also focus buttons that Enter then fires.
        core.set_keyboard_nav(false);
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let left = flags
            .left
            .and_then(|p| p.canonicalize().ok())
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| home.clone());
        let mut app = Self {
            core,
            panes: [
                Tabs::new(Tab::new(1, left.clone())),
                Tabs::new(Tab::new(2, home.clone())),
            ],
            scroll_ids: [widget::Id::unique(), widget::Id::unique()],
            active: 0,
            tz: TimeZone::system(),
            next_id: 2,
            mods: Modifiers::empty(),
            dialog: None,
            input_id: widget::Id::unique(),
        };
        let task = Task::batch([app.load(0, left, None), app.load(1, home, None)]);
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(action) => {
                if let Some(d) = &self.dialog {
                    // Modal: panels must not move. Enter confirms a dialog without a text field.
                    if action == Action::Enter && matches!(d, Dialog::ConfirmDelete { .. }) {
                        return self.submit_dialog();
                    }
                    return Task::none();
                }
                return self.act(self.active, action);
            }
            Message::Listed {
                side,
                tab,
                path,
                focus,
                result,
            } => {
                let Some(t) = self.panes[side]
                    .items_mut()
                    .iter_mut()
                    .find(|t| t.id == tab)
                else {
                    return Task::none(); // tab was closed
                };
                if t.pending.as_ref() != Some(&path) {
                    return Task::none(); // stale: the user has moved on
                }
                t.pending = None;
                match result {
                    Ok(entries) => {
                        t.error = None;
                        t.panel.set_listing(path, entries, focus.as_deref());
                        return self.reveal(side, tab);
                    }
                    Err(e) => t.error = Some(e),
                }
            }
            Message::Click(side, i) => {
                self.active = side;
                let panel = &mut self.panes[side].active_mut().panel;
                panel.set_cursor(i);
                if self.mods.control() {
                    panel.toggle_mark();
                }
            }
            Message::DoubleClick(side, i) => {
                self.active = side;
                self.panes[side].active_mut().panel.set_cursor(i);
                return self.act(side, Action::Enter);
            }
            Message::Header(side, key) => {
                self.active = side;
                return self.act(side, Action::Sort(key));
            }
            Message::Scrolled(side, offset, height) => {
                let t = self.panes[side].active_mut();
                t.offset = offset;
                t.height = height;
            }
            Message::SelectTab(side, i) => {
                self.active = side;
                self.panes[side].select(i);
                return self.restore_scroll(side);
            }
            Message::CloseTabAt(side, i) => {
                self.panes[side].close(i);
                return self.restore_scroll(side);
            }
            Message::Modifiers(m) => self.mods = m,
            Message::DialogInput(s) => {
                if let Some(input) = self.dialog.as_mut().and_then(Dialog::input_mut) {
                    *input = s;
                }
            }
            Message::DialogSubmit => return self.submit_dialog(),
            Message::DialogCancel => self.dialog = None,
            Message::Exit => return cosmic::iced::exit(),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(route_event)
    }

    fn view(&self) -> Element<'_, Message> {
        crate::view::view(self)
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        self.dialog
            .as_ref()
            .map(|d| dialogs::view(d, &self.input_id))
    }

    fn footer(&self) -> Option<Element<'_, Message>> {
        Some(crate::view::fkey_bar())
    }
}

impl App {
    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Scan `path` for the active tab of `side` in the background; the result lands in `Message::Listed`.
    fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
        let t = self.panes[side].active_mut();
        t.pending = Some(path.clone());
        let tab = t.id;
        let show_hidden = t.panel.show_hidden();
        Task::perform(
            async move {
                let p = path.clone();
                let result = tokio::task::spawn_blocking(move || listing::scan(&p, show_hidden))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                Message::Listed {
                    side,
                    tab,
                    path,
                    focus,
                    result,
                }
            },
            cosmic::Action::App,
        )
    }

    fn act(&mut self, side: usize, action: Action) -> Task<Message> {
        if let Some(d) = self.dialog_for(side, action) {
            let focus = matches!(d, Dialog::Mask { .. } | Dialog::Input { .. });
            self.dialog = Some(d);
            return if focus {
                widget::text_input::focus(self.input_id.clone())
            } else {
                Task::none()
            };
        }
        let t = self.panes[side].active_mut();
        let page = viewport::page_rows(ROW_H, t.height) as isize;
        let tab = t.id;
        let panel = &mut t.panel;
        match action {
            Action::SwitchPane => {
                self.active = 1 - self.active;
                return Task::none();
            }
            Action::Up => panel.move_cursor(-1),
            Action::Down => panel.move_cursor(1),
            Action::PageUp => panel.move_cursor(-page),
            Action::PageDown => panel.move_cursor(page),
            Action::Home => panel.cursor_home(),
            Action::End => panel.cursor_end(),
            Action::Sort(key) => panel.set_sort(key),
            Action::Enter => {
                if let Some((path, focus)) = panel.enter_path() {
                    return self.load(side, path, focus);
                }
                let file = panel.current().map(|e| panel.cwd().join(&e.os_name));
                if let Some(file) = file
                    && let Err(err) = open_detached(&file)
                {
                    t.error = Some(fl!("open-failed", err = err.to_string()));
                }
            }
            Action::Parent => {
                if let Some((path, focus)) = panel.parent_path() {
                    return self.load(side, path, Some(focus));
                }
            }
            Action::Root => return self.load(side, "/".into(), None),
            Action::Mark => panel.toggle_mark(),
            Action::MarkDown => panel.toggle_mark_and_move(1),
            Action::MarkUp => panel.toggle_mark_and_move(-1),
            Action::Invert => panel.invert(),
            Action::SelectAll => panel.mark_all(true),
            Action::UnselectAll => panel.mark_all(false),
            Action::Cancel => {}
            Action::Copy
            | Action::Move
            | Action::Rename
            | Action::Mkdir
            | Action::Delete
            | Action::DeletePermanent => {}
            // Opened by `dialog_for` above.
            Action::SelectGroup | Action::UnselectGroup => {}
            Action::Reload => {
                let cwd = panel.cwd().to_path_buf();
                return self.load(side, cwd, None);
            }
            Action::NewTab => {
                let id = self.next_id();
                let copy = self.panes[side].active().duplicate(id);
                self.panes[side].open_after(copy);
                return self.restore_scroll(side);
            }
            Action::CloseTab => {
                let i = self.panes[side].active_index();
                self.panes[side].close(i);
                return self.restore_scroll(side);
            }
            Action::NextTab => {
                self.panes[side].next();
                return self.restore_scroll(side);
            }
            Action::PrevTab => {
                self.panes[side].prev();
                return self.restore_scroll(side);
            }
        }
        self.reveal(side, tab)
    }

    /// The dialog an action opens, if any (nothing when there is nothing to act on).
    fn dialog_for(&self, side: usize, action: Action) -> Option<Dialog> {
        let panel = &self.panes[side].active().panel;
        let input = |op, sources, input| Dialog::Input {
            op,
            side,
            sources,
            input,
        };
        match action {
            Action::SelectGroup | Action::UnselectGroup => Some(Dialog::Mask {
                side,
                select: action == Action::SelectGroup,
                input: "*".into(),
            }),
            Action::Copy | Action::Move => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let op = if action == Action::Copy {
                    InputOp::Copy
                } else {
                    InputOp::Move
                };
                Some(input(
                    op,
                    sources,
                    dir_input(self.panes[1 - side].active().panel.cwd()),
                ))
            }
            Action::Mkdir => Some(input(InputOp::Mkdir, Vec::new(), String::new())),
            Action::Rename => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                Some(input(
                    InputOp::Rename,
                    vec![panel.cwd().join(&e.os_name)],
                    e.name.clone(),
                ))
            }
            Action::Delete | Action::DeletePermanent => {
                let paths = panel.targets();
                (!paths.is_empty()).then_some(Dialog::ConfirmDelete {
                    side,
                    permanent: action == Action::DeletePermanent,
                    paths,
                })
            }
            _ => None,
        }
    }

    fn submit_dialog(&mut self) -> Task<Message> {
        let Some(d) = self.dialog.take() else {
            return Task::none();
        };
        match d {
            Dialog::Mask {
                side,
                select,
                input,
            } => {
                self.panes[side]
                    .active_mut()
                    .panel
                    .mark_by_mask(&Mask::parse(&input), select);
                Task::none()
            }
            Dialog::Input {
                op: InputOp::Mkdir,
                side,
                input,
                ..
            } => self.mkdir(side, input.trim()),
            Dialog::Input {
                op,
                side,
                sources,
                input,
            } => self.start_transfer(op, side, sources, input.trim()),
            Dialog::ConfirmDelete {
                side,
                permanent,
                paths,
            } => self.start_delete(side, permanent, paths),
        }
    }

    /// F7: create (possibly nested) dirs, then put the cursor on the first created component.
    fn mkdir(&mut self, side: usize, name: &str) -> Task<Message> {
        if name.is_empty() {
            return Task::none();
        }
        let t = self.panes[side].active_mut();
        let cwd = t.panel.cwd().to_path_buf();
        match std::fs::create_dir_all(cwd.join(name)) {
            Ok(()) => {
                let focus = match Path::new(name).components().next() {
                    Some(Component::Normal(first)) => Some(first.to_string_lossy().into_owned()),
                    _ => None,
                };
                self.load(side, cwd, focus)
            }
            Err(e) => {
                t.error = Some(e.to_string());
                Task::none()
            }
        }
    }

    // ponytail: stubs until the job runner lands (next task).
    fn start_transfer(
        &mut self,
        _op: InputOp,
        _side: usize,
        _sources: Vec<PathBuf>,
        _input: &str,
    ) -> Task<Message> {
        Task::none()
    }

    fn start_delete(
        &mut self,
        _side: usize,
        _permanent: bool,
        _paths: Vec<PathBuf>,
    ) -> Task<Message> {
        Task::none()
    }

    /// Keep tab `tab`'s cursor row fully visible. An inactive tab only gets its offset updated;
    /// `restore_scroll` applies it when that tab is shown.
    fn reveal(&mut self, side: usize, tab: u64) -> Task<Message> {
        let is_active = self.panes[side].active().id == tab;
        let Some(t) = self.panes[side]
            .items_mut()
            .iter_mut()
            .find(|t| t.id == tab)
        else {
            return Task::none();
        };
        match viewport::scroll_to_cursor(t.panel.cursor(), ROW_H, t.offset, t.height) {
            Some(y) => {
                t.offset = y;
                if is_active {
                    self.restore_scroll(side)
                } else {
                    Task::none()
                }
            }
            None => Task::none(),
        }
    }

    /// Scroll the pane's list to the active tab's stored offset.
    fn restore_scroll(&self, side: usize) -> Task<Message> {
        scrollable::scroll_to(
            self.scroll_ids[side].clone(),
            AbsoluteOffset {
                x: None,
                y: Some(self.panes[side].active().offset),
            },
        )
    }
}

/// Window events → messages. Panel keys only when no widget took the event (`Ignored`).
fn route_event(
    event: cosmic::iced::Event,
    status: event::Status,
    _window: cosmic::iced::window::Id,
) -> Option<Message> {
    match event {
        // Any status: a focused text_input captures Escape to unfocus itself, and the dialog must still close.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            modifiers,
            ..
        }) if modifiers.is_empty() => Some(Message::DialogCancel),
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if status == event::Status::Ignored => {
            keymap::action(&key, physical_key, modifiers).map(Message::Key)
        }
        cosmic::iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::Modifiers(m))
        }
        _ => None,
    }
}

/// Default F5/F6 target: the other pane's dir with a trailing `/` (so it reads as "into this dir").
fn dir_input(dir: &Path) -> String {
    let s = dir.display().to_string();
    if s.ends_with('/') { s } else { s + "/" }
}

/// `xdg-open` without blocking the UI or leaving a zombie.
fn open_detached(path: &std::path::Path) -> std::io::Result<()> {
    let mut child = std::process::Command::new("xdg-open").arg(path).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::keyboard::key::{Code, Named, Physical};
    use cosmic::iced::keyboard::{Key, Location};

    fn press(named: Named, code: Code, status: event::Status) -> Option<Message> {
        let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(named),
            modified_key: Key::Named(named),
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        });
        route_event(event, status, cosmic::iced::window::Id::unique())
    }

    #[test]
    fn escape_closes_dialog_even_when_text_input_captured_it() {
        // libcosmic's text_input captures Escape (to unfocus itself); the dialog must still close.
        let msg = press(Named::Escape, Code::Escape, event::Status::Captured);
        assert!(matches!(msg, Some(Message::DialogCancel)), "{msg:?}");
    }

    #[test]
    fn captured_keys_do_not_reach_the_panels() {
        let msg = press(Named::ArrowDown, Code::ArrowDown, event::Status::Captured);
        assert!(msg.is_none(), "{msg:?}");
        let msg = press(Named::ArrowDown, Code::ArrowDown, event::Status::Ignored);
        assert!(matches!(msg, Some(Message::Key(Action::Down))), "{msg:?}");
    }
}
