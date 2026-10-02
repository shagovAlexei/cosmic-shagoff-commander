use crate::fl;
use crate::keymap::{self, Action};
use cosmic::app::{Core, Task};
use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
use cosmic::iced::{Subscription, event, keyboard};
use cosmic::{Application, Element, widget};
use shagoff_core::format::TimeZone;
use shagoff_core::listing::{self, Entry};
use shagoff_core::panel::Panel;
use shagoff_core::sort::SortKey;
use shagoff_core::viewport;
use std::path::PathBuf;

pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";
/// Fixed row height of the file list; the viewport math depends on it.
pub const ROW_H: f32 = 22.0;
// ponytail: list height is guessed until the scrollable reports its bounds (it does on the first event).
const FALLBACK_LIST_H: f32 = 400.0;

pub struct Flags {
    pub left: Option<PathBuf>,
}

pub struct Pane {
    pub panel: Panel,
    pub scroll_id: widget::Id,
    pub offset: f32,
    pub height: f32,
    /// Path of the scan in flight; results for any other path are stale.
    pending: Option<PathBuf>,
    pub error: Option<String>,
}

impl Pane {
    fn new(cwd: PathBuf) -> Self {
        Self {
            panel: Panel::new(cwd),
            scroll_id: widget::Id::unique(),
            offset: 0.0,
            height: FALLBACK_LIST_H,
            pending: None,
            error: None,
        }
    }
}

pub struct App {
    core: Core,
    pub panes: [Pane; 2],
    pub active: usize,
    pub tz: TimeZone,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key(Action),
    Listed {
        side: usize,
        path: PathBuf,
        focus: Option<String>,
        result: Result<Vec<Entry>, String>,
    },
    Click(usize, usize),
    DoubleClick(usize, usize),
    Header(usize, SortKey),
    /// side, scroll offset y, viewport height
    Scrolled(usize, f32, f32),
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
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let left = flags
            .left
            .and_then(|p| p.canonicalize().ok())
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| home.clone());
        let mut app = Self {
            core,
            panes: [Pane::new(left.clone()), Pane::new(home.clone())],
            active: 0,
            tz: TimeZone::system(),
        };
        let task = Task::batch([app.load(0, left, None), app.load(1, home, None)]);
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(action) => return self.act(self.active, action),
            Message::Listed {
                side,
                path,
                focus,
                result,
            } => {
                let pane = &mut self.panes[side];
                if pane.pending.as_ref() != Some(&path) {
                    return Task::none(); // stale: the user has moved on
                }
                pane.pending = None;
                match result {
                    Ok(entries) => {
                        pane.error = None;
                        pane.panel.set_listing(path, entries, focus.as_deref());
                        return self.reveal(side);
                    }
                    Err(e) => pane.error = Some(e),
                }
            }
            Message::Click(side, i) => {
                self.active = side;
                self.panes[side].panel.set_cursor(i);
            }
            Message::DoubleClick(side, i) => {
                self.active = side;
                self.panes[side].panel.set_cursor(i);
                return self.act(side, Action::Enter);
            }
            Message::Header(side, key) => {
                self.active = side;
                return self.act(side, Action::Sort(key));
            }
            Message::Scrolled(side, offset, height) => {
                self.panes[side].offset = offset;
                self.panes[side].height = height;
            }
            Message::Exit => return cosmic::iced::exit(),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(|event, status, _| match event {
            cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) if status == event::Status::Ignored => {
                keymap::action(&key, physical_key, modifiers).map(Message::Key)
            }
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Message> {
        crate::view::view(self)
    }

    fn footer(&self) -> Option<Element<'_, Message>> {
        Some(crate::view::fkey_bar())
    }
}

impl App {
    /// Scan `path` in the background; the result lands in `Message::Listed`.
    fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
        let pane = &mut self.panes[side];
        pane.pending = Some(path.clone());
        let show_hidden = pane.panel.show_hidden();
        Task::perform(
            async move {
                let p = path.clone();
                let result = tokio::task::spawn_blocking(move || listing::scan(&p, show_hidden))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                Message::Listed {
                    side,
                    path,
                    focus,
                    result,
                }
            },
            cosmic::Action::App,
        )
    }

    fn act(&mut self, side: usize, action: Action) -> Task<Message> {
        let page = viewport::page_rows(ROW_H, self.panes[side].height) as isize;
        let panel = &mut self.panes[side].panel;
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
                    self.panes[side].error = Some(fl!("open-failed", err = err.to_string()));
                }
            }
            Action::Parent => {
                if let Some((path, focus)) = panel.parent_path() {
                    return self.load(side, path, Some(focus));
                }
            }
            Action::Root => return self.load(side, "/".into(), None),
            Action::Reload => {
                let cwd = panel.cwd().to_path_buf();
                return self.load(side, cwd, None);
            }
        }
        self.reveal(side)
    }

    /// Scroll so the cursor row is fully visible.
    fn reveal(&mut self, side: usize) -> Task<Message> {
        let pane = &mut self.panes[side];
        match viewport::scroll_to_cursor(pane.panel.cursor(), ROW_H, pane.offset, pane.height) {
            Some(y) => {
                pane.offset = y;
                scrollable::scroll_to(
                    pane.scroll_id.clone(),
                    AbsoluteOffset {
                        x: None,
                        y: Some(y),
                    },
                )
            }
            None => Task::none(),
        }
    }
}

/// `xdg-open` without blocking the UI or leaving a zombie.
fn open_detached(path: &std::path::Path) -> std::io::Result<()> {
    let mut child = std::process::Command::new("xdg-open").arg(path).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
