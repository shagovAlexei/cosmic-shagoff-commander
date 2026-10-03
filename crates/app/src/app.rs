use crate::clip;
use crate::config::{self, Config, HotEntry, LastTab, State};
use crate::dialogs::{
    self, Dialog, FindField, InputOp, ListItem, ListKind, MrField, SyncOpt, Toggle,
};
use crate::fl;
use crate::jobs::{self, Job};
use crate::keymap::{self, Action};
use cosmic::app::{Core, Task};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::keyboard::Modifiers;
use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
use cosmic::iced::{Subscription, event, keyboard};
use cosmic::{Application, Element, widget};
use shagoff_core::archive::{self, Format};
use shagoff_core::clipboard::Kind as ClipKind;
use shagoff_core::drives::{self, Drive};
use shagoff_core::format::{self, TimeZone};
use shagoff_core::history::History;
use shagoff_core::launch;
use shagoff_core::listing::{self, Entry};
use shagoff_core::mask::Mask;
use shagoff_core::multirename::{self, Case, Rule};
use shagoff_core::ops::{self, ErrorChoice, Method, PlanError, Report, Resolution};
use shagoff_core::panel::{self, PARENT, Panel};
use shagoff_core::session::{self, PaneState};
use shagoff_core::sort::SortKey;
use shagoff_core::tabs::Tabs;
use shagoff_core::viewport;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

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
    /// Generation and path of the scan in flight; any other result is stale.
    pub(crate) pending: Option<(u64, PathBuf)>,
    pub error: Option<String>,
    /// Alt+← / Alt+→ / Alt+↓.
    pub(crate) history: History,
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
            history: History::default(),
        }
    }

    /// Where the tab is going: the dir being scanned, else the shown one. Rescans use this so they
    /// never cancel a navigation still in flight.
    fn target(&self) -> PathBuf {
        match &self.pending {
            Some((_, p)) => p.clone(),
            None => self.panel.cwd().to_path_buf(),
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
            history: self.history.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
    Pack,
    Unpack,
    Extract,
    Sync,
}

/// A file operation in progress (the progress dialog's data).
pub struct Running {
    pub side: usize,
    pub kind: OpKind,
    pub done: u64,
    pub total: u64,
    pub current: String,
    cancel: Arc<AtomicBool>,
    /// Name to put the source pane's cursor on afterwards (rename).
    focus: Option<String>,
    /// Enter / F3 in an archive: argv to run on the extracted file once the job succeeds.
    open: Option<(Vec<OsString>, PathBuf)>,
}

/// Quick search (Alt+letter) or filter (Ctrl+S) field, shown instead of the pane's status line.
pub struct Search {
    pub side: usize,
    pub text: String,
    pub filter: bool,
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
    job: Option<Running>,
    /// Id of the dialog text field (one field at a time), for focusing it on open.
    pub(crate) input_id: widget::Id,
    /// Quick search / filter field, if open.
    pub search: Option<Search>,
    pub home: PathBuf,
    pub config: Config,
    config_handler: Option<cosmic_config::Config>,
    state_handler: Option<cosmic_config::Config>,
    /// Last state written, to skip identical writes.
    saved: State,
    /// Alt+F7 settings, kept in `State`.
    find: config::FindPrefs,
    pub drives: Vec<Drive>,
    /// (free, total) bytes of each pane's current disk.
    pub space: [Option<(u64, u64)>; 2],
}

#[derive(Debug, Clone)]
pub enum Message {
    Key(Action),
    Listed {
        tab: u64,
        generation: u64,
        path: PathBuf,
        focus: Option<String>,
        result: Result<Vec<Entry>, String>,
        /// (free, total) of the scanned dir's filesystem; statvfs can block on network mounts.
        space: Option<(u64, u64)>,
    },
    Click(usize, usize),
    DoubleClick(usize, usize),
    Header(usize, SortKey),
    /// side, scroll offset y, viewport height (of the active tab)
    Scrolled(usize, f32, f32),
    /// side, real viewport height of the pane's list (from a sensor: on_scroll misses resizes)
    Resized(usize, f32),
    SelectTab(usize, usize),
    CloseTabAt(usize, usize),
    Modifiers(Modifiers),
    DialogInput(String),
    DialogSubmit,
    DialogCancel,
    MrInput(MrField, String),
    MrCase(Case),
    /// Pack dialog: format button.
    PackFormat(Format),
    /// Pack / unpack dialog checkboxes.
    Toggle(Toggle),
    FindInput(FindField, String),
    FindCase,
    FindStart,
    /// Enter in a find field: go to the selected result after ↑/↓, else search.
    FindSubmit,
    FindStop,
    Find(crate::find::FindEvent),
    /// Go to result i of the find dialog.
    FindPick(usize),
    SyncOpt(SyncOpt),
    SyncCompare,
    SyncStop,
    /// Compare result for the sync dialog with this id.
    SyncCompared(u64, Vec<shagoff_core::sync::Row>),
    /// Cycle row i's arrow: → ← none.
    SyncFlip(usize),
    SyncRun,
    /// Compare result for the diff dialog with this id.
    DiffReady(u64, Arc<Result<shagoff_core::diff::Outcome, String>>),
    DiffNext,
    DiffPrev,
    Op(jobs::Event),
    Resolve(Resolution),
    ErrorAnswer(ErrorChoice),
    CancelJob,
    /// A panel key a focused text field captured (F-keys, PgUp/PgDn, Insert, Ctrl+…).
    FieldKey(Action),
    /// Click on entry i of the open list dialog.
    ListPick(usize),
    /// Text typed into the quick search / filter field.
    SearchInput(String),
    /// Files read from the system clipboard by Ctrl+V (`None`: no files there).
    Pasted(Option<(ClipKind, Vec<PathBuf>)>),
    /// Enter in that field.
    SearchSubmit,
    /// The watched dir of this pane's active tab changed.
    Changed(usize),
    /// Drive button / drive list entry: (side, drive root). A path, not an index: the list can change.
    Drive(usize, PathBuf),
    Config(Config),
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
        let (ch, sh) = (config::config_handler(), config::state_handler());
        let (cfg, state): (Config, State) = (config::read(ch.as_ref()), config::read(sh.as_ref()));
        let saved = state.clone();
        archive::clean_temp(&archive::temp_root()); // left by crashed runs; not in `build` (tests)
        let (mut app, task) = Self::build(core, cfg, state, flags.left, home);
        app.saved = saved;
        (app.config_handler, app.state_handler) = (ch, sh);
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.save_state();
        task
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            event::listen_with(route_event),
            self.core()
                .watch_config::<Config>(APP_ID)
                .map(|u| Message::Config(u.config)),
        ];
        if self.job.is_none() {
            // Paused during file operations: finish_job rescans both panes anyway.
            for side in 0..2 {
                let cwd = self.panes[side].active().panel.cwd().to_path_buf();
                subs.push(crate::watcher::watch(side, cwd));
            }
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        crate::view::view(self)
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        if let Some(d) = &self.dialog {
            return Some(dialogs::view(d, &self.input_id, &self.tz));
        }
        self.job.as_ref().map(dialogs::progress)
    }

    fn footer(&self) -> Option<Element<'_, Message>> {
        Some(crate::view::fkey_bar())
    }
}

impl App {
    /// Everything but the disk-backed config handlers (tests use this directly).
    pub fn build(
        core: Core,
        config: Config,
        state: State,
        left: Option<PathBuf>,
        home: PathBuf,
    ) -> (Self, Task<Message>) {
        let home_fallback = home.clone();
        let mut app = Self {
            core,
            panes: [
                Tabs::new(Tab::new(0, home.clone())),
                Tabs::new(Tab::new(0, home.clone())),
            ],
            scroll_ids: [widget::Id::unique(), widget::Id::unique()],
            active: state.active.min(1),
            tz: TimeZone::system(),
            next_id: 0,
            mods: Modifiers::empty(),
            dialog: None,
            job: None,
            input_id: widget::Id::unique(),
            search: None,
            home,
            config,
            config_handler: None,
            state_handler: None,
            saved: State::default(),
            find: state.find.clone(),
            drives: Vec::new(),
            space: [None, None],
        };
        // A file opens its folder; a missing path keeps the saved tab.
        let left = left
            .and_then(|p| p.canonicalize().ok())
            .map(|p| session::existing_dir(&p, &home_fallback));
        for side in 0..2 {
            let (mut paths, active) = session::restore(&state.panes[side], &app.home);
            if side == 0
                && let Some(l) = &left
            {
                paths[active] = l.clone();
            }
            let mut tabs = Tabs::new(app.new_tab(paths[0].clone()));
            for p in &paths[1..] {
                tabs.open_after(app.new_tab(p.clone()));
            }
            tabs.select(active);
            app.panes[side] = tabs;
        }
        app.refresh_mounts();
        let task = app.load_all();
        (app, task)
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(action) => {
                if let Some(Dialog::List { kind, cursor, .. }) = &self.dialog
                    && *kind == ListKind::Hotlist
                    && *cursor >= 1
                    && matches!(action, Action::Delete | Action::DeletePermanent)
                {
                    let i = *cursor - 1;
                    return self.hotlist_remove(i);
                }
                if let Some(Dialog::List { cursor, items, .. }) = &mut self.dialog {
                    match action {
                        Action::Up => *cursor = cursor.saturating_sub(1),
                        Action::Down => *cursor = (*cursor + 1).min(items.len().saturating_sub(1)),
                        Action::Enter => {
                            let i = *cursor;
                            return self.pick(i);
                        }
                        _ => {}
                    }
                    return Task::none();
                }
                if let Some(Dialog::Diff(_)) = &self.dialog {
                    return match action {
                        Action::Down => self.diff_step(1),
                        Action::Up => self.diff_step(-1),
                        _ => Task::none(),
                    };
                }
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    match action {
                        Action::Up => {
                            f.cursor = f.cursor.saturating_sub(1);
                            f.in_list = true;
                        }
                        Action::Down => {
                            f.cursor = (f.cursor + 1).min(f.results.len().saturating_sub(1));
                            f.in_list = true;
                        }
                        Action::Enter => {
                            let i = f.cursor;
                            return self.find_pick(i);
                        }
                        Action::SwitchPane => return cosmic::iced::widget::operation::focus_next(),
                        _ => {}
                    }
                    return Task::none();
                }
                if let Some(d) = &self.dialog {
                    // Modal: panels must not move. Enter confirms a dialog without a text field.
                    if action == Action::Enter && matches!(d, Dialog::ConfirmDelete { .. }) {
                        return self.submit_dialog();
                    }
                    // Tab is not taken by text fields; walk the multi-rename form with it.
                    if action == Action::SwitchPane && matches!(d, Dialog::MultiRename(_)) {
                        return cosmic::iced::widget::operation::focus_next();
                    }
                    return Task::none();
                }
                if self.job.is_some() {
                    return Task::none(); // panels wait for the running operation
                }
                if let Some(s) = &self.search {
                    let (side, filter, text) = (s.side, s.filter, s.text.clone());
                    match action {
                        Action::Up | Action::Down if !filter => {
                            let p = &self.panes[side].active().panel;
                            let n = p.entries().len();
                            let down = action == Action::Down;
                            let from = if down {
                                p.cursor() + 1
                            } else {
                                p.cursor() + n - 1
                            };
                            if let Some(i) = p.find(&text, from, down) {
                                let t = self.panes[side].active_mut();
                                t.panel.set_cursor(i);
                                let tab = t.id;
                                return self.reveal(side, tab);
                            }
                            return Task::none();
                        }
                        Action::Up | Action::Down => {} // filter: plain cursor move, field stays
                        _ => self.search = None,        // any other key closes the field and acts
                    }
                }
                return self.act(self.active, action);
            }
            Message::Listed {
                tab,
                generation,
                path,
                focus,
                result,
                space,
            } => {
                // The tab may have moved (Ctrl+U): find it by id; `side` is only where it started.
                let Some(side) =
                    (0..2).find(|&s| self.panes[s].items().iter().any(|t| t.id == tab))
                else {
                    return Task::none(); // tab was closed
                };
                let Some(t) = self.panes[side]
                    .items_mut()
                    .iter_mut()
                    .find(|t| t.id == tab)
                else {
                    return Task::none();
                };
                if t.pending.as_ref().map(|(g, _)| *g) != Some(generation) {
                    return Task::none(); // stale: the user has moved on
                }
                t.pending = None;
                match result {
                    Ok(entries) => {
                        t.error = None;
                        t.history.visit(&path); // no-op for a rescan of the current entry
                        t.panel.set_listing(path, entries, focus.as_deref());
                        if self.panes[side].active().id == tab {
                            self.refresh_mounts();
                            self.space[side] = space;
                        }
                        return self.reveal(side, tab);
                    }
                    Err(e) => {
                        t.error = Some(fl!(
                            "list-failed",
                            path = path.display().to_string(),
                            err = e
                        ))
                    }
                }
            }
            Message::Click(side, i) => {
                self.search = None;
                self.active = side;
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                if self.mods.control() {
                    t.panel.toggle_mark();
                }
                let tab = t.id;
                return self.reveal(side, tab); // a half-visible row scrolls fully in
            }
            Message::DoubleClick(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].active_mut().panel.set_cursor(i);
                return self.act(side, Action::Enter);
            }
            Message::Header(side, key) => {
                self.search = None;
                self.active = side;
                return self.act(side, Action::Sort(key));
            }
            Message::Scrolled(side, offset, height) => {
                // Wheel / scrollbar: the view moves freely, the cursor stays where it is.
                let t = self.panes[side].active_mut();
                t.offset = offset;
                t.height = height;
            }
            Message::Resized(side, height) => {
                // One list widget per pane: every tab shares its viewport height.
                let shrunk = height < self.panes[side].active().height;
                for t in self.panes[side].items_mut() {
                    t.height = height;
                }
                if shrunk {
                    let tab = self.panes[side].active().id;
                    return self.reveal(side, tab); // keep the cursor on screen
                }
            }
            Message::SelectTab(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].select(i);
                return self.tab_switched(side);
            }
            Message::CloseTabAt(side, i) => {
                self.search = None;
                self.panes[side].close(i);
                return self.tab_switched(side);
            }
            Message::Modifiers(m) => self.mods = m,
            Message::DialogInput(s) => {
                if let Some(input) = self.dialog.as_mut().and_then(Dialog::input_mut) {
                    *input = s;
                }
            }
            Message::MrInput(field, s) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    *match field {
                        MrField::Name => &mut m.rule.name,
                        MrField::Ext => &mut m.rule.ext,
                        MrField::Find => &mut m.rule.find,
                        MrField::Replace => &mut m.rule.replace,
                        MrField::Start => &mut m.start,
                        MrField::Step => &mut m.step,
                        MrField::Digits => &mut m.digits,
                    } = s;
                }
            }
            Message::MrCase(c) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.case = c;
                }
            }
            Message::PackFormat(f) => {
                if let Some(Dialog::Pack(p)) = &mut self.dialog {
                    p.format = f;
                    p.path = archive::with_format(&p.path, f);
                }
                self.save_pack_format(f);
            }
            Message::Toggle(t) => match (&mut self.dialog, t) {
                (Some(Dialog::Pack(p)), Toggle::MoveAfter) => p.move_after = !p.move_after,
                (Some(Dialog::Pack(p)), Toggle::Separate) => p.separate = !p.separate,
                (Some(Dialog::Unpack { own_dir, .. }), Toggle::OwnDir) => *own_dir = !*own_dir,
                _ => {}
            },
            Message::FindInput(field, s) => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.in_list = false;
                    *match field {
                        FindField::Mask => &mut f.mask,
                        FindField::Dir => &mut f.dir,
                        FindField::Text => &mut f.text,
                    } = s;
                }
            }
            Message::FindCase => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.case_sensitive = !f.case_sensitive;
                    f.in_list = false;
                }
            }
            Message::FindStart => return self.start_find(),
            Message::FindSubmit => {
                if let Some(Dialog::Find(f)) = &self.dialog
                    && f.in_list
                    && !f.results.is_empty()
                {
                    let i = f.cursor;
                    return self.find_pick(i);
                }
                return self.start_find();
            }
            Message::FindStop => {
                if let Some(Dialog::Find(f)) = &self.dialog
                    && let Some(s) = &f.stop
                {
                    s.store(true, Ordering::Relaxed);
                }
            }
            Message::Find(e) => self.on_find_event(e),
            Message::FindPick(i) => return self.find_pick(i),
            Message::SyncOpt(o) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog {
                    let flag = match o {
                        SyncOpt::Recursive => &mut s.recursive,
                        SyncOpt::Content => &mut s.content,
                        SyncOpt::IgnoreDate => &mut s.ignore_date,
                        SyncOpt::ShowSame => &mut s.show_same,
                    };
                    *flag = !*flag;
                    // The rows must match the options they are synced with.
                    if o != SyncOpt::ShowSame {
                        return self.start_compare();
                    }
                }
            }
            Message::SyncCompare => return self.start_compare(),
            Message::SyncStop => {
                if let Some(Dialog::Sync(s)) = &self.dialog
                    && let Some(f) = &s.running
                {
                    f.store(true, Ordering::Relaxed);
                }
            }
            Message::SyncCompared(id, rows) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && s.id == id
                {
                    s.rows = rows;
                    s.running = None;
                }
            }
            Message::SyncFlip(i) => {
                use shagoff_core::sync::Dir;
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && let Some(row) = s.rows.get_mut(i)
                {
                    row.dir = match row.dir {
                        Dir::ToRight => Dir::ToLeft,
                        Dir::ToLeft => Dir::None,
                        Dir::None => Dir::ToRight,
                    };
                }
            }
            Message::SyncRun => return self.start_sync(),
            Message::DiffReady(id, out) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.id == id
                {
                    d.result = Some(out);
                    d.block = 0;
                    return self.diff_scroll();
                }
            }
            Message::DiffNext => return self.diff_step(1),
            Message::DiffPrev => return self.diff_step(-1),
            Message::DialogSubmit => return self.submit_dialog(),
            Message::DialogCancel => match self.dialog.take() {
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(Resolution::Cancel);
                }
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(ErrorChoice::Cancel);
                }
                Some(_) => {}
                None => {
                    if let Some(s) = self.search.take() {
                        if s.filter {
                            self.panes[s.side].active_mut().panel.set_filter(None);
                        }
                    } else if self.job.is_some() {
                        self.cancel_job();
                    } else {
                        self.panes[self.active].active_mut().panel.set_filter(None);
                    }
                }
            },
            Message::Op(event) => return self.on_job_event(event),
            Message::Pasted(Some((kind, paths)))
                if !paths.is_empty() && self.job.is_none() && self.dialog.is_none() =>
            {
                let op = match kind {
                    ClipKind::Copy => InputOp::Copy,
                    ClipKind::Cut => InputOp::Move,
                };
                // "": `cwd.join("")` ends with `/`, so `plan` always puts the files inside cwd.
                let task = self.start_transfer(op, self.active, paths, "");
                if kind == ClipKind::Cut && self.job.is_some() {
                    return Task::batch([clip::clear(), task]);
                }
                return task;
            }
            Message::Pasted(_) => {}
            Message::Resolve(r) => match self.dialog.take() {
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(r);
                }
                other => self.dialog = other,
            },
            Message::ErrorAnswer(c) => match self.dialog.take() {
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(c);
                }
                other => self.dialog = other,
            },
            Message::CancelJob => self.cancel_job(),
            Message::SearchInput(text) => {
                let Some(s) = &self.search else {
                    return Task::none();
                };
                let (side, filter) = (s.side, s.filter);
                let t = self.panes[side].active_mut();
                if filter {
                    t.panel.set_filter(Some(text.clone()));
                } else if !text.is_empty() {
                    match t.panel.find(&text, 0, true) {
                        Some(i) => t.panel.set_cursor(i),
                        None => return Task::none(), // rejected: the field keeps the old text
                    }
                }
                let tab = t.id;
                if let Some(s) = &mut self.search {
                    s.text = text;
                }
                return self.reveal(side, tab);
            }
            // Only the quick search field forwards keys; a dialog's text field keeps its own.
            Message::ListPick(i) => return self.pick(i),
            Message::FieldKey(action) => {
                if self.search.is_some() && self.dialog.is_none() {
                    return self.handle(Message::Key(action));
                }
            }
            Message::SearchSubmit => {
                if let Some(s) = self.search.take()
                    && !s.filter
                {
                    return self.act(s.side, Action::Enter);
                }
            }
            Message::Changed(side) => {
                let t = self.panes[side].active();
                // A change in the dir we are leaving must not cancel the scan of the one we enter.
                if self.job.is_none() && t.target() == t.panel.cwd() {
                    let cwd = t.target();
                    return self.load(side, cwd, None);
                }
            }
            Message::Drive(side, path) => {
                self.search = None;
                if self.job.is_none() {
                    self.dialog = None;
                    return self.go_to(side, path);
                }
            }
            Message::Config(c) => {
                let hidden_changed = c.show_hidden != self.config.show_hidden;
                self.config = c;
                if hidden_changed {
                    return self.apply_hidden();
                }
            }
            Message::Exit => {
                let mine = archive::temp_root().join(std::process::id().to_string());
                let _ = std::fs::remove_dir_all(mine);
                return cosmic::iced::exit();
            }
        }
        Task::none()
    }

    fn new_tab(&mut self, path: PathBuf) -> Tab {
        let mut t = Tab::new(self.next_id(), path);
        t.panel.set_show_hidden(self.config.show_hidden);
        t
    }

    /// Rescan every tab of both panes (startup, Ctrl+H).
    fn load_all(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for side in 0..2 {
            for i in 0..self.panes[side].items().len() {
                let dir = self.panes[side].items()[i].target();
                tasks.push(self.load_tab(side, i, dir, None));
            }
        }
        Task::batch(tasks)
    }

    fn apply_hidden(&mut self) -> Task<Message> {
        let on = self.config.show_hidden;
        for side in 0..2 {
            for t in self.panes[side].items_mut() {
                t.panel.set_show_hidden(on);
            }
        }
        self.load_all()
    }

    /// Write tab paths when they changed (cheap compare on every message; no write if equal).
    fn save_state(&mut self) {
        let state = State {
            panes: [0, 1].map(|s| PaneState {
                tabs: self.panes[s]
                    .items()
                    .iter()
                    .map(|t| t.panel.cwd().to_path_buf())
                    .collect(),
                active: self.panes[s].active_index(),
            }),
            active: self.active,
            find: self.find.clone(),
        };
        if state == self.saved {
            return;
        }
        if let Some(h) = &self.state_handler
            && let Err(e) = state.write_entry(h)
        {
            log::warn!("state: {e}");
        }
        self.saved = state;
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Scan `path` for the active tab of `side` in the background; the result lands in `Message::Listed`.
    fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
        let i = self.panes[side].active_index();
        self.load_tab(side, i, path, focus)
    }

    /// Scan `path` for tab `i` of `side` in the background.
    fn load_tab(
        &mut self,
        side: usize,
        i: usize,
        path: PathBuf,
        focus: Option<String>,
    ) -> Task<Message> {
        let generation = self.next_id();
        let t = &mut self.panes[side].items_mut()[i];
        t.pending = Some((generation, path.clone()));
        let tab = t.id;
        let show_hidden = t.panel.show_hidden();
        Task::perform(
            async move {
                let p = path.clone();
                let (result, space) = tokio::task::spawn_blocking(move || {
                    let result = listing::scan(&p, show_hidden).map_err(|e| e.to_string());
                    (result, drives::space(&p))
                })
                .await
                .unwrap_or_else(|e| (Err(e.to_string()), None));
                Message::Listed {
                    tab,
                    generation,
                    path,
                    focus,
                    result,
                    space,
                }
            },
            cosmic::Action::App,
        )
    }

    fn act(&mut self, side: usize, action: Action) -> Task<Message> {
        // An error stays in the status line until the next action in that pane.
        self.panes[side].active_mut().error = None;
        match action {
            Action::QuickSearch(c) => return self.quick_search(side, c),
            Action::QuickFilter => return self.quick_filter(side),
            _ => {}
        }
        if self.read_only(side, action) {
            self.panes[side].active_mut().error = Some(fl!("archive-read-only"));
            return Task::none();
        }
        if let Some(d) = self.dialog_for(side, action) {
            if matches!(d, Dialog::Sync(_)) {
                self.dialog = Some(d);
                return self.start_compare(); // TC compares right away
            }
            if matches!(d, Dialog::Diff(_)) {
                self.dialog = Some(d);
                return self.start_diff();
            }
            let focus = matches!(
                d,
                Dialog::Mask { .. }
                    | Dialog::Input { .. }
                    | Dialog::MultiRename(_)
                    | Dialog::Pack(_)
                    | Dialog::Unpack { .. }
                    | Dialog::Find(_)
            );
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
        let target = t.target();
        let loading = target != t.panel.cwd();
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
            // The rows on screen belong to the dir being left: entering one would undo the navigation.
            Action::Enter if loading => {}
            Action::Enter => {
                if let Some((path, focus)) = panel.enter_path() {
                    return self.load(side, path, focus);
                }
                let cwd = panel.cwd().to_path_buf();
                let current = panel.current().map(|e| (e.os_name.clone(), e.name.clone()));
                if let Some((os_name, name)) = current {
                    match archive::split_path(&cwd) {
                        Some((arc, inner)) => {
                            return self.open_from_archive(
                                side,
                                arc,
                                inner,
                                os_name,
                                &[],
                                &["xdg-open"],
                            );
                        }
                        // Not inside one already: archives inside archives open as files.
                        None if Format::detect(&name).is_some_and(Format::is_tree) => {
                            return self.load(side, cwd.join(os_name), None);
                        }
                        None => {}
                    }
                }
                let t = self.panes[side].active_mut();
                let panel = &t.panel;
                let file = panel.current().map(|e| panel.cwd().join(&e.os_name));
                if let Some(file) = file
                    && let Err(err) = spawn_detached(&launch::command(&[], &["xdg-open"], &file))
                {
                    t.error = Some(fl!("open-failed", err = err.to_string()));
                }
            }
            // From where the tab is going, so quick Backspaces on a slow fs are not lost.
            Action::Parent => {
                if let Some((path, focus)) = panel::parent_of(&target) {
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
            Action::ToggleHidden => {
                let on = !self.config.show_hidden;
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_show_hidden(h, on) {
                            log::warn!("config: {e}");
                        }
                    }
                    None => self.config.show_hidden = on,
                }
                return self.apply_hidden();
            }
            Action::View | Action::Edit => {
                let cwd = panel.cwd().to_path_buf();
                let current = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| e.os_name.clone());
                if let (Some(os_name), Some((arc, inner))) = (current, archive::split_path(&cwd)) {
                    // Edit is refused by `read_only` before getting here.
                    let viewer = self.config.viewer.clone();
                    return self.open_from_archive(
                        side,
                        arc,
                        inner,
                        os_name,
                        &viewer,
                        &["xdg-open"],
                    );
                }
                let t = self.panes[side].active_mut();
                let panel = &t.panel;
                let file = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| (panel.cwd().join(&e.os_name), e.name.clone()));
                if let Some((file, name)) = file {
                    if !file.exists() {
                        t.error = Some(fl!("broken-link", name = name));
                        return Task::none();
                    }
                    let argv = if action == Action::View {
                        launch::command(&self.config.viewer, &["xdg-open"], &file)
                    } else {
                        launch::command(&self.config.editor, &["cosmic-edit"], &file)
                    };
                    if let Err(err) = spawn_detached(&argv) {
                        t.error = Some(fl!("open-failed", err = err.to_string()));
                    }
                }
            }
            // Opened by `dialog_for` above.
            Action::Drives(_) => {}
            Action::QuickSearch(_) | Action::QuickFilter => {} // handled above
            Action::HistoryBack | Action::HistoryForward => {
                let history = &mut self.panes[side].active_mut().history;
                let step = if action == Action::HistoryBack {
                    history.back()
                } else {
                    history.forward()
                };
                if let Some(path) = step {
                    return self.load(side, path, None);
                }
            }
            // Reached only when `dialog_for` found no pair of files.
            Action::CompareFiles => t.error = Some(fl!("diff-pick-two")),
            Action::CompareLists => {
                let (l, r) = shagoff_core::sync::compare_lists(
                    self.panes[0].active().panel.entries(),
                    self.panes[1].active().panel.entries(),
                );
                // Nothing to mark must not look like a key that did nothing.
                if l.is_empty() && r.is_empty() {
                    self.panes[side].active_mut().error = Some(fl!("compare-identical"));
                }
                self.panes[0].active_mut().panel.mark_names(&l);
                self.panes[1].active_mut().panel.mark_names(&r);
                return Task::none();
            }
            Action::SwapPanes => {
                self.panes.swap(0, 1);
                self.space.swap(0, 1);
                self.search = None;
                return Task::batch([self.restore_scroll(0), self.restore_scroll(1)]);
            }
            // Opened by `dialog_for` above.
            Action::HistoryList | Action::Hotlist => {}
            Action::ClipCopy if archive::split_path(panel.cwd()).is_some() => {
                // No real paths to put on the clipboard: extract into the other panel right away.
                let paths = panel.targets();
                let cwd = panel.cwd().to_path_buf();
                let dest = self.panes[1 - side].active().target();
                return self.start_extract(side, &cwd, &paths, dest);
            }
            Action::ClipCopy | Action::ClipCut => {
                let paths = panel.targets();
                if !paths.is_empty() {
                    let kind = if action == Action::ClipCut {
                        ClipKind::Cut
                    } else {
                        ClipKind::Copy
                    };
                    return clip::put(kind, &paths);
                }
            }
            Action::ClipPaste => return clip::take(),
            Action::Copy
            | Action::Move
            | Action::Rename
            | Action::MultiRename
            | Action::Pack
            | Action::Unpack
            | Action::FindFiles
            | Action::SyncDirs
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
                if !self.panes[side].close(i) {
                    if self.config.last_tab_close == LastTab::Home {
                        let home = match &self.config.home_dir {
                            Some(d) => session::existing_dir(
                                &session::expand_home(d, &self.home),
                                &self.home,
                            ),
                            None => self.home.clone(),
                        };
                        return self.load(side, home, None);
                    }
                    return Task::none();
                }
                return self.tab_switched(side);
            }
            Action::NextTab => {
                self.panes[side].next();
                return self.tab_switched(side);
            }
            Action::PrevTab => {
                self.panes[side].prev();
                return self.tab_switched(side);
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
            Action::Hotlist => Some(Dialog::List {
                kind: ListKind::Hotlist,
                side,
                cursor: 0,
                items: self.hotlist_items(),
            }),
            Action::HistoryList => Some(Dialog::List {
                kind: ListKind::History,
                side,
                cursor: 0,
                items: self.panes[side]
                    .active()
                    .history
                    .recent()
                    .into_iter()
                    .map(|p| ListItem {
                        label: format::dir_title(&p),
                        path: p,
                    })
                    .collect(),
            }),
            // A copy: mounts are re-read on every listing and must not shift under the cursor.
            Action::Drives(s) => Some(Dialog::List {
                kind: ListKind::Drives,
                side: s,
                cursor: drives::containing(&self.drives, self.panes[s].active().panel.cwd())
                    .unwrap_or(0),
                items: self
                    .drives
                    .iter()
                    .map(|d| ListItem {
                        label: d.label.clone(),
                        path: d.path.clone(),
                    })
                    .collect(),
            }),
            Action::MultiRename => {
                let wanted: HashSet<PathBuf> = panel.targets().into_iter().collect();
                // A non-UTF-8 name can't round-trip through the text masks; leave it out.
                let files: Vec<(String, SystemTime)> = panel
                    .entries()
                    .iter()
                    .filter(|e| {
                        e.os_name.to_str().is_some()
                            && wanted.contains(&panel.cwd().join(&e.os_name))
                    })
                    .map(|e| (e.name.clone(), e.mtime))
                    .collect();
                if files.is_empty() {
                    return None;
                }
                let dir = panel.cwd().to_path_buf();
                // Hidden files count too, even when not shown; fall back to the listing.
                let taken = multirename::other_names(&dir, &files).unwrap_or_else(|_| {
                    panel
                        .entries()
                        .iter()
                        .filter(|e| e.name != PARENT && !files.iter().any(|(n, _)| *n == e.name))
                        .map(|e| e.name.clone())
                        .collect()
                });
                let current = panel
                    .current()
                    .filter(|e| files.iter().any(|(n, _)| *n == e.name))
                    .map(|e| e.name.clone());
                Some(Dialog::MultiRename(Box::new(dialogs::MultiRename {
                    side,
                    dir,
                    files,
                    taken,
                    current,
                    rule: Rule::default(),
                    start: "1".into(),
                    step: "1".into(),
                    digits: "1".into(),
                })))
            }
            Action::Pack => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let format = Format::from_ext(&self.config.pack_format).unwrap_or(Format::Zip);
                let base = panel.cwd().to_path_buf();
                let name = archive::archive_name(&sources, &base, format);
                let path = self.panes[1 - side].active().panel.cwd().join(name);
                Some(Dialog::Pack(Box::new(dialogs::Pack {
                    side,
                    sources,
                    base,
                    path: path.display().to_string(),
                    format,
                    move_after: false,
                    separate: false,
                })))
            }
            Action::FindFiles => {
                // Inside an archive: search where the archive lies.
                let cwd = panel.cwd();
                let dir = match archive::split_path(cwd) {
                    Some((a, _)) => a.parent().map_or(cwd.to_path_buf(), Path::to_path_buf),
                    None => cwd.to_path_buf(),
                };
                Some(Dialog::Find(Box::new(dialogs::Find {
                    side,
                    mask: self.find.mask.clone(),
                    dir: dir.display().to_string(),
                    text: self.find.text.clone(),
                    case_sensitive: self.find.case_sensitive,
                    results: Vec::new(),
                    total: 0,
                    current: String::new(),
                    cursor: 0,
                    id: 0, // set by each search start
                    stop: None,
                    in_list: false,
                })))
            }
            Action::CompareFiles => {
                let (left, right) = self.diff_pair(side)?;
                Some(Dialog::Diff(Box::new(dialogs::DiffDlg {
                    left,
                    right,
                    id: 0,
                    result: None,
                    block: 0,
                    scroll: widget::Id::unique(),
                })))
            }
            Action::SyncDirs => Some(Dialog::Sync(Box::new(dialogs::SyncDlg {
                side,
                left: self.panes[0].active().target(),
                right: self.panes[1].active().target(),
                recursive: true,
                content: false,
                ignore_date: false,
                hidden: panel.show_hidden(),
                show_same: false,
                rows: Vec::new(),
                id: 0,
                running: None,
            }))),
            Action::Unpack => {
                let archives: Vec<PathBuf> = panel
                    .targets()
                    .into_iter()
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| Format::detect(&n.to_string_lossy()).is_some())
                    })
                    .collect();
                (!archives.is_empty()).then(|| Dialog::Unpack {
                    side,
                    archives,
                    path: dir_input(self.panes[1 - side].active().panel.cwd()),
                    own_dir: false,
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
            Dialog::MultiRename(m) => {
                let rows = m.rows(&self.tz);
                if rows.iter().any(|r| r.problem.is_some()) {
                    self.dialog = Some(Dialog::MultiRename(m)); // Enter does nothing until fixed
                    return Task::none();
                }
                let pairs = multirename::plan(&m.dir, &rows);
                if pairs.is_empty() {
                    return Task::none();
                }
                let focus = m
                    .current
                    .as_ref()
                    .and_then(|c| rows.iter().find(|r| &r.old == c))
                    .map(|r| r.new.clone());
                let job = Job::Transfer {
                    method: Method::Rename,
                    pairs,
                };
                self.start_job(m.side, OpKind::Move, job, focus)
            }
            Dialog::Pack(p) => {
                let cwd = self.panes[p.side].active().target();
                let path = cwd.join(p.path.trim());
                if into_archive(&path) {
                    self.panes[p.side].active_mut().error = Some(fl!("archive-read-only"));
                    return Task::none();
                }
                let groups = archive::groups(&p.sources, &p.base, &path, p.format, p.separate);
                // Cursor on the new archive when it lands in this panel.
                let focus = match groups.as_slice() {
                    [(_, a)] if a.parent() == Some(cwd.as_path()) => {
                        a.file_name().map(|n| n.to_string_lossy().into_owned())
                    }
                    _ => None,
                };
                let job = Job::Pack {
                    format: p.format,
                    base: p.base,
                    groups,
                    move_after: p.move_after,
                };
                self.start_job(p.side, OpKind::Pack, job, focus)
            }
            Dialog::Unpack {
                side,
                archives,
                path,
                own_dir,
            } => {
                let dest = self.panes[side].active().target().join(path.trim());
                if into_archive(&dest) {
                    self.panes[side].active_mut().error = Some(fl!("archive-read-only"));
                    return Task::none();
                }
                let job = Job::Unpack {
                    archives,
                    dest,
                    own_dir,
                };
                self.start_job(side, OpKind::Unpack, job, None)
            }
            d @ Dialog::Sync(_) => {
                self.dialog = Some(d);
                self.start_sync()
            }
            // Read-only view: Enter does nothing.
            d @ Dialog::Diff(_) => {
                self.dialog = Some(d);
                Task::none()
            }
            d @ Dialog::Find(_) => {
                self.dialog = Some(d);
                self.start_find()
            }
            d @ Dialog::List { .. } => {
                let i = match &d {
                    Dialog::List { cursor, .. } => *cursor,
                    _ => 0,
                };
                self.dialog = Some(d);
                self.pick(i)
            }
            // Answered with their own buttons, not Enter/OK.
            d @ (Dialog::Conflict { .. } | Dialog::Error { .. }) => {
                self.dialog = Some(d);
                Task::none()
            }
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
                t.error = Some(fl!("mkdir-failed", path = name, err = e.to_string()));
                Task::none()
            }
        }
    }

    /// F5 / F6 / rename: resolve the target, then run the transfer on a worker thread.
    fn start_transfer(
        &mut self,
        op: InputOp,
        side: usize,
        sources: Vec<PathBuf>,
        input: &str,
    ) -> Task<Message> {
        // While a navigation is in flight the rows still show the dir being left; aim at where the tab is going.
        let cwd = self.panes[side].active().target();
        if op != InputOp::Rename && into_archive(&cwd.join(input)) {
            self.panes[side].active_mut().error = Some(fl!("archive-read-only"));
            return Task::none();
        }
        // From the rows' own dir: right after Enter on an archive they still show the dir left.
        let from = sources
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        if let Some(from) = from
            && op == InputOp::Copy
            && archive::split_path(&from).is_some()
        {
            return self.start_extract(side, &from, &sources, cwd.join(input));
        }
        let (method, kind) = match op {
            InputOp::Copy => (Method::Copy, OpKind::Copy),
            _ => (Method::Move, OpKind::Move),
        };
        let (planned, focus) = if op == InputOp::Rename {
            // Not `plan`: an existing dir name must be refused, not moved into or merged with.
            let Some(src) = sources.first() else {
                return Task::none();
            };
            (ops::rename_pairs(src, input), Some(input.to_string()))
        } else {
            (ops::plan(&sources, &cwd.join(input)), None)
        };
        let pairs = match planned {
            Ok(pairs) if pairs.is_empty() => return Task::none(), // rename to the same name
            Ok(pairs) => pairs,
            Err(e) => {
                self.panes[side].active_mut().error = Some(plan_error(&e));
                return Task::none();
            }
        };
        self.start_job(side, kind, Job::Transfer { method, pairs }, focus)
    }

    /// Ctrl+Shift+D: two marked files of this panel, else the files under both cursors.
    fn diff_pair(&self, side: usize) -> Option<(PathBuf, PathBuf)> {
        let file = |s: usize| {
            let p = &self.panes[s].active().panel;
            p.current()
                .filter(|e| !e.is_dir() && e.name != PARENT)
                .map(|e| p.cwd().join(&e.os_name))
        };
        let panel = &self.panes[side].active().panel;
        let marked: Vec<PathBuf> = panel
            .entries()
            .iter()
            .filter(|e| panel.is_marked(e) && !e.is_dir())
            .map(|e| panel.cwd().join(&e.os_name))
            .collect();
        match marked.as_slice() {
            [a, b] => Some((a.clone(), b.clone())),
            _ => Some((file(0)?, file(1)?)),
        }
    }

    fn start_diff(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Diff(d)) = &mut self.dialog else {
            return Task::none();
        };
        d.id = id;
        let (a, b) = (d.left.clone(), d.right.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    shagoff_core::diff::compare(&a, &b).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |out| cosmic::Action::App(Message::DiffReady(id, Arc::new(out))),
        )
    }

    /// Next / previous block of differences, scrolled into view.
    fn diff_step(&mut self, delta: isize) -> Task<Message> {
        let Some(Dialog::Diff(d)) = &mut self.dialog else {
            return Task::none();
        };
        let n = d.blocks().len();
        if n == 0 {
            return Task::none();
        }
        d.block = d.block.saturating_add_signed(delta).min(n - 1);
        self.diff_scroll()
    }

    fn diff_scroll(&self) -> Task<Message> {
        let Some(Dialog::Diff(d)) = &self.dialog else {
            return Task::none();
        };
        let Some(&row) = d.blocks().get(d.block) else {
            return Task::none();
        };
        // A couple of rows of context above the block.
        let y = row.saturating_sub(2) as f32 * dialogs::DIFF_ROW_H;
        scrollable::scroll_to(
            d.scroll.clone(),
            AbsoluteOffset {
                x: Some(0.0),
                y: Some(y),
            },
        )
    }

    /// (Re)compare the sync dialog's dirs in the background; an older compare is stopped.
    fn start_compare(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Sync(s)) = &mut self.dialog else {
            return Task::none();
        };
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = s.running.replace(stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        s.id = id;
        let (l, r, o) = (s.left.clone(), s.right.clone(), s.options());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || shagoff_core::sync::compare(&l, &r, &o, &stop))
                    .await
                    .unwrap_or_default()
            },
            move |rows| cosmic::Action::App(Message::SyncCompared(id, rows)),
        )
    }

    /// Copy by the arrows; the dialog closes, both panes reload after the job.
    fn start_sync(&mut self) -> Task<Message> {
        let Some(Dialog::Sync(s)) = &self.dialog else {
            return Task::none();
        };
        if s.running.is_some() {
            return Task::none();
        }
        let (to_right, to_left) = shagoff_core::sync::plan(&s.left, &s.right, &s.rows);
        if to_right.is_empty() && to_left.is_empty() {
            return Task::none();
        }
        let side = s.side;
        self.dialog = None;
        self.start_job(side, OpKind::Sync, Job::Sync { to_right, to_left }, None)
    }

    /// (Re)start the search of the open find dialog; the previous one is stopped.
    fn start_find(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let side = f.side;
        self.find = config::FindPrefs {
            mask: f.mask.clone(),
            text: f.text.clone(),
            case_sensitive: f.case_sensitive,
        };
        let root = self.panes[side].active().target().join(f.dir.trim());
        let text = f.text.trim().to_string();
        let q = shagoff_core::search::Query {
            mask: Mask::parse(&f.mask),
            text: (!text.is_empty()).then_some(text),
            case_sensitive: f.case_sensitive,
            hidden: self.panes[side].active().panel.show_hidden(),
        };
        let (stop, events) = crate::find::spawn(id, root, q);
        if let Some(Dialog::Find(f)) = &mut self.dialog {
            if let Some(old) = f.stop.replace(stop) {
                old.store(true, Ordering::Relaxed);
            }
            (f.id, f.cursor, f.total, f.in_list) = (id, 0, 0, false);
            f.results.clear();
        }
        Task::run(events, |e| cosmic::Action::App(Message::Find(e)))
    }

    fn on_find_event(&mut self, e: crate::find::FindEvent) {
        use crate::find::FindEvent;
        let Some(Dialog::Find(f)) = &mut self.dialog else {
            return;
        };
        match e {
            FindEvent::Found(id, paths) if id == f.id => {
                f.total += paths.len();
                let room = dialogs::FIND_SHOWN.saturating_sub(f.results.len());
                f.results.extend(paths.into_iter().take(room));
            }
            FindEvent::Dir(id, d) if id == f.id => f.current = d.display().to_string(),
            FindEvent::Done(id) if id == f.id => {
                f.stop = None;
                f.current.clear();
            }
            _ => {} // a previous search
        }
    }

    /// Result i: its dir in the dialog's panel, cursor on it (also for a found dir, as in TC).
    fn find_pick(&mut self, i: usize) -> Task<Message> {
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let (side, Some(path)) = (f.side, f.results.get(i).cloned()) else {
            return Task::none();
        };
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Task::none();
        };
        let (dir, name) = (dir.to_path_buf(), name.to_string_lossy().into_owned());
        self.dialog = None; // stops the search (Drop)
        self.active = side;
        self.load(side, dir, Some(name))
    }

    /// Copy `paths` (inside the archive dir `cwd`) out into the real dir `dest`.
    fn start_extract(
        &mut self,
        side: usize,
        cwd: &Path,
        paths: &[PathBuf],
        dest: PathBuf,
    ) -> Task<Message> {
        let Some((archive, inner)) = archive::split_path(cwd) else {
            return Task::none();
        };
        if paths.is_empty() || into_archive(&dest) {
            return Task::none();
        }
        let names = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(PathBuf::from)
            .collect();
        let job = Job::Extract {
            archive,
            inner,
            names,
            dest,
        };
        self.start_job(side, OpKind::Extract, job, None)
    }

    /// Enter / F3 on a file inside an archive: extract it to a fresh temp dir, then open it.
    fn open_from_archive(
        &mut self,
        side: usize,
        archive: PathBuf,
        inner: PathBuf,
        name: OsString,
        prog: &[String],
        default: &[&str],
    ) -> Task<Message> {
        let dir = match archive::fresh_temp_dir(&archive::temp_root()) {
            Ok(d) => d,
            Err(e) => {
                self.panes[side].active_mut().error = Some(fl!("open-failed", err = e.to_string()));
                return Task::none();
            }
        };
        let file = dir.join(&name);
        let argv = launch::command(prog, default, &file);
        let job = Job::Extract {
            archive,
            inner,
            names: vec![PathBuf::from(name)],
            dest: dir,
        };
        let task = self.start_job(side, OpKind::Extract, job, None);
        if let Some(j) = &mut self.job {
            j.open = Some((argv, file));
        }
        task
    }

    /// Actions that would change an archive (this panel inside one, or the other one as target).
    fn read_only(&self, side: usize, action: Action) -> bool {
        let inside = |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
        match action {
            Action::Edit
            | Action::Rename
            | Action::MultiRename
            | Action::Mkdir
            | Action::Delete
            | Action::DeletePermanent
            | Action::ClipCut
            | Action::ClipPaste => inside(side),
            Action::Move | Action::Pack | Action::Unpack | Action::SyncDirs => {
                inside(side) || inside(1 - side)
            }
            Action::Copy => inside(1 - side),
            Action::ClipCopy => inside(side) && inside(1 - side),
            _ => false,
        }
    }

    fn start_delete(&mut self, side: usize, permanent: bool, paths: Vec<PathBuf>) -> Task<Message> {
        self.start_job(side, OpKind::Delete, Job::Delete { paths, permanent }, None)
    }

    fn start_job(
        &mut self,
        side: usize,
        kind: OpKind,
        job: Job,
        focus: Option<String>,
    ) -> Task<Message> {
        let (cancel, events) = jobs::spawn(job);
        self.job = Some(Running {
            side,
            kind,
            done: 0,
            total: 0,
            current: String::new(),
            cancel,
            focus,
            open: None,
        });
        Task::run(events, |e| cosmic::Action::App(Message::Op(e)))
    }

    fn cancel_job(&self) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn on_job_event(&mut self, event: jobs::Event) -> Task<Message> {
        match event {
            jobs::Event::Progress {
                done,
                total,
                current,
            } => {
                if let Some(j) = &mut self.job {
                    (j.done, j.total) = (done, total);
                    j.current = current
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
            }
            jobs::Event::Conflict { src, dst, reply } => {
                self.dialog = Some(Dialog::Conflict { src, dst, reply });
            }
            jobs::Event::Error { path, error, reply } => {
                self.dialog = Some(Dialog::Error { path, error, reply });
            }
            jobs::Event::Finished(report) => return self.finish_job(&report),
        }
        Task::none()
    }

    /// Unmark what was processed and reload both panes (cursor on the renamed entry, if any).
    fn finish_job(&mut self, report: &Report) -> Task<Message> {
        let Some(job) = self.job.take() else {
            return Task::none();
        };
        let side = job.side;
        if let Some((argv, file)) = &job.open
            && !report.cancelled
            && file.exists()
            && let Err(err) = spawn_detached(argv)
        {
            self.panes[side].active_mut().error = Some(fl!("open-failed", err = err.to_string()));
        }
        self.panes[side]
            .active_mut()
            .panel
            .unmark(&report.completed);
        let here = self.panes[side].active().panel.cwd().to_path_buf();
        let there = self.panes[1 - side].active().panel.cwd().to_path_buf();
        Task::batch([
            self.load(side, here, job.focus),
            self.load(1 - side, there, None),
        ])
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

    /// Alt+letter: append to an open search (or start one) if the text still matches something.
    fn quick_search(&mut self, side: usize, c: char) -> Task<Message> {
        let mut text = match &self.search {
            Some(s) if s.side == side && !s.filter => s.text.clone(),
            _ => String::new(),
        };
        text.push(c);
        let Some(i) = self.panes[side].active().panel.find(&text, 0, true) else {
            return Task::none(); // TC: a letter that finds nothing is not taken
        };
        self.search = Some(Search {
            side,
            text,
            filter: false,
        });
        self.active = side;
        let t = self.panes[side].active_mut();
        t.panel.set_cursor(i);
        let tab = t.id;
        Task::batch([
            widget::text_input::focus(self.input_id.clone()),
            self.reveal(side, tab),
        ])
    }

    fn quick_filter(&mut self, side: usize) -> Task<Message> {
        let text = self.panes[side]
            .active()
            .panel
            .filter()
            .unwrap_or_default()
            .to_string();
        self.search = Some(Search {
            side,
            text,
            filter: true,
        });
        self.active = side;
        widget::text_input::focus(self.input_id.clone())
    }

    /// Mounts change rarely and procfs never blocks: re-read with every listing.
    fn refresh_mounts(&mut self) {
        if let Ok(m) = std::fs::read_to_string("/proc/self/mounts") {
            self.drives = drives::parse(&m, &self.home);
        }
    }

    /// Enter / click on entry `i` of the open list.
    fn pick(&mut self, i: usize) -> Task<Message> {
        let Some(Dialog::List {
            kind, side, items, ..
        }) = self.dialog.take()
        else {
            return Task::none();
        };
        if kind == ListKind::Hotlist && i == 0 {
            let cwd = self.panes[side].active().panel.cwd().to_path_buf();
            if !self.config.hotlist.iter().any(|e| e.path == cwd) {
                let mut list = self.config.hotlist.clone();
                list.push(HotEntry {
                    name: format::dir_title(&cwd),
                    path: cwd,
                });
                self.save_hotlist(list);
            }
            return Task::none();
        }
        match items.get(i) {
            Some(item) => self.go_to(side, item.path.clone()),
            None => Task::none(),
        }
    }

    /// Hotlist rows: "add current dir" (empty path), then the favourites.
    fn hotlist_items(&self) -> Vec<ListItem> {
        std::iter::once(ListItem {
            label: fl!("hotlist-add"),
            path: PathBuf::new(),
        })
        .chain(self.config.hotlist.iter().map(|e| ListItem {
            label: e.name.clone(),
            path: e.path.clone(),
        }))
        .collect()
    }

    fn save_pack_format(&mut self, f: Format) {
        let v = f.ext().to_string();
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_pack_format(h, v) {
                    log::warn!("config: {e}");
                }
            }
            None => self.config.pack_format = v,
        }
    }

    fn save_hotlist(&mut self, list: Vec<HotEntry>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_hotlist(h, list) {
                    log::warn!("config: {e}");
                }
            }
            None => self.config.hotlist = list,
        }
    }

    /// Delete on hotlist entry `i`: drop it, save, refresh the open list in place.
    fn hotlist_remove(&mut self, i: usize) -> Task<Message> {
        let mut list = self.config.hotlist.clone();
        if i < list.len() {
            list.remove(i);
            self.save_hotlist(list);
        }
        let fresh = self.hotlist_items();
        if let Some(Dialog::List { items, cursor, .. }) = &mut self.dialog {
            *cursor = (*cursor).min(fresh.len() - 1);
            *items = fresh;
        }
        Task::none()
    }

    fn go_to(&mut self, side: usize, path: PathBuf) -> Task<Message> {
        self.active = side;
        self.load(side, path, None)
    }

    /// The newly shown tab was not watched while hidden: restore its scroll and rescan it.
    fn tab_switched(&mut self, side: usize) -> Task<Message> {
        let dir = self.panes[side].active().target();
        Task::batch([self.restore_scroll(side), self.load(side, dir, None)])
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
        // A focused text field captures every key but Up/Down/Tab; pass on the ones it has no use for.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if not_for_text(&key, physical_key, modifiers) => {
            keymap::action(&key, physical_key, modifiers).map(Message::FieldKey)
        }
        cosmic::iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::Modifiers(m))
        }
        _ => None,
    }
}

/// Keys a text field captures but does not edit with: F-keys, PgUp/PgDn, Insert and Ctrl combos
/// other than text editing and the clipboard (Ctrl+A/C/V/X/Z, Ctrl+arrows/Backspace/Delete/Home/End).
fn not_for_text(key: &keyboard::Key, physical: keyboard::key::Physical, mods: Modifiers) -> bool {
    use keyboard::key::{Code, Named, Physical};
    match key {
        keyboard::Key::Named(n) => {
            let editing = matches!(
                n,
                Named::Backspace
                    | Named::Delete
                    | Named::Home
                    | Named::End
                    | Named::ArrowLeft
                    | Named::ArrowRight
                    | Named::Enter
                    | Named::Escape
            );
            matches!(
                n,
                Named::F1
                    | Named::F2
                    | Named::F3
                    | Named::F4
                    | Named::F5
                    | Named::F6
                    | Named::F7
                    | Named::F8
                    | Named::F9
                    | Named::F10
                    | Named::F11
                    | Named::F12
                    | Named::PageUp
                    | Named::PageDown
                    | Named::Insert
            ) || (mods.control() && !editing)
        }
        _ => {
            let clipboard = matches!(
                physical,
                Physical::Code(Code::KeyA | Code::KeyC | Code::KeyV | Code::KeyX | Code::KeyZ)
            );
            mods.control() && !clipboard
        }
    }
}

fn plan_error(e: &PlanError) -> String {
    match e {
        PlanError::IntoItself(p) => fl!("plan-into-itself", path = p.display().to_string()),
        PlanError::SameFile(p) => fl!("plan-same-file", path = p.display().to_string()),
        PlanError::BadName => fl!("rename-bad-name"),
        PlanError::Exists(p) => fl!("rename-exists", path = p.display().to_string()),
        PlanError::Empty => String::new(),
    }
}

/// Default F5/F6 target: the other pane's dir with a trailing `/` (so it reads as "into this dir").
/// A typed target that lies inside an archive (`a.zip/x`). The archive file itself is not: packing
/// or copying onto an existing `a.zip` asks to replace it, as for any file.
fn into_archive(p: &Path) -> bool {
    archive::split_path(p).is_some_and(|(_, inner)| !inner.as_os_str().is_empty())
}

fn dir_input(dir: &Path) -> String {
    let s = dir.display().to_string();
    if s.ends_with('/') { s } else { s + "/" }
}

/// Run `argv` without blocking the UI or leaving a zombie.
fn spawn_detached(argv: &[OsString]) -> std::io::Result<()> {
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut child = std::process::Command::new(prog).args(args).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LastTab, State};
    use crate::dialogs::ListKind;
    use cosmic::iced::keyboard::key::{Code, Named, Physical};
    use cosmic::iced::keyboard::{Key, Location};
    use shagoff_core::archive::Format;
    use shagoff_core::drives::Drive;
    use shagoff_core::multirename::Case;
    use shagoff_core::session::PaneState;

    fn app_with(config: Config, state: State) -> App {
        App::build(Core::default(), config, state, None, std::env::temp_dir()).0
    }

    fn cwds(app: &App, side: usize) -> Vec<PathBuf> {
        app.panes[side]
            .items()
            .iter()
            .map(|t| t.panel.cwd().to_path_buf())
            .collect()
    }

    #[test]
    fn spawn_detached_reports_missing_program() {
        let argv = [
            std::ffi::OsString::from("/nonexistent/shagoff-test-prog"),
            "x".into(),
        ];
        assert!(spawn_detached(&argv).is_err());
    }

    #[test]
    fn drive_dialog_navigates_and_opens() {
        let mut app = app_with(Config::default(), State::default());
        app.drives = vec![
            Drive {
                label: "/".into(),
                path: "/".into(),
            },
            Drive {
                label: "tmp".into(),
                path: std::env::temp_dir(),
            },
        ];
        let _ = app.update(Message::Key(Action::Drives(1)));
        assert!(matches!(
            app.dialog,
            Some(Dialog::List {
                kind: ListKind::Drives,
                side: 1,
                ..
            })
        ));
        let _ = app.update(Message::Key(Action::Up));
        let _ = app.update(Message::Key(Action::Up)); // clamped at 0
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down)); // clamped at 1
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 1, .. })));
        app.panes[1].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::Enter));
        assert!(app.dialog.is_none());
        assert_eq!(app.active, 1);
        assert!(app.panes[1].active().pending.is_some());
    }

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            os_name: name.into(),
            ext: String::new(),
            size: 0,
            mtime: std::time::UNIX_EPOCH,
            kind: shagoff_core::listing::Kind::File,
            is_link: false,
            mode: 0o644,
        }
    }

    #[test]
    fn regression_second_scan_of_same_path_wins() {
        let mut app = app_with(Config::default(), State::default());
        let (id, cwd) = (
            app.panes[0].active().id,
            app.panes[0].active().panel.cwd().to_path_buf(),
        );
        let _ = app.load(0, cwd.clone(), None);
        let first = app.panes[0].active().pending.as_ref().unwrap().0;
        let _ = app.load(0, cwd.clone(), None);
        let second = app.panes[0].active().pending.as_ref().unwrap().0;
        let listed = |generation, name: &str| Message::Listed {
            tab: id,
            generation,
            path: cwd.clone(),
            focus: None,
            result: Ok(vec![entry(name)]),
            space: None,
        };
        let _ = app.update(listed(second, "new"));
        let _ = app.update(listed(first, "old")); // late, stale
        let names: Vec<_> = app.panes[0]
            .active()
            .panel
            .entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert!(
            names.contains(&"new".to_string()) && !names.contains(&"old".to_string()),
            "{names:?}"
        );
    }

    /// Start navigating pane 0 to a fresh temp dir; returns (dir guard, tab id, generation).
    fn navigating(app: &mut App) -> (tempfile::TempDir, u64, u64) {
        let tmp = tempfile::tempdir().unwrap();
        let _ = app.load(0, tmp.path().into(), None);
        let t = app.panes[0].active();
        let generation = t.pending.as_ref().unwrap().0;
        (tmp, t.id, generation)
    }

    fn listed_ok(id: u64, generation: u64, path: &Path) -> Message {
        Message::Listed {
            tab: id,
            generation,
            path: path.into(),
            focus: None,
            result: Ok(vec![]),
            space: Some((1, 2)),
        }
    }

    #[test]
    fn regression_watcher_does_not_cancel_navigation() {
        let mut app = app_with(Config::default(), State::default());
        let (tmp, id, generation) = navigating(&mut app);
        let _ = app.update(Message::Changed(0)); // old dir changed while the new one loads
        let _ = app.update(listed_ok(id, generation, tmp.path()));
        assert_eq!(app.panes[0].active().panel.cwd(), tmp.path());
    }

    #[test]
    fn regression_ctrl_h_keeps_pending_navigation() {
        let mut app = app_with(Config::default(), State::default());
        let (tmp, _, _) = navigating(&mut app);
        let _ = app.update(Message::Key(Action::ToggleHidden));
        let pending = app.panes[0]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone());
        assert_eq!(pending.as_deref(), Some(tmp.path()));
    }

    #[test]
    fn listed_brings_free_space_of_the_active_tab() {
        let mut app = app_with(Config::default(), State::default());
        let (tmp, id, generation) = navigating(&mut app);
        let _ = app.update(listed_ok(id, generation, tmp.path()));
        assert_eq!(app.space[0], Some((1, 2)));
    }

    #[test]
    fn build_restores_tabs_and_active_pane() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        std::fs::create_dir(&a).unwrap();
        let state = State {
            panes: [
                PaneState {
                    tabs: vec![tmp.path().into(), a.clone()],
                    active: 1,
                },
                PaneState {
                    tabs: vec![tmp.path().join("gone")],
                    active: 0,
                },
            ],
            active: 1,
            ..State::default()
        };
        let app = app_with(Config::default(), state);
        assert_eq!(cwds(&app, 0), [tmp.path().to_path_buf(), a]);
        assert_eq!(app.panes[0].active_index(), 1);
        assert_eq!(cwds(&app, 1), [tmp.path().to_path_buf()]);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn argv_path_replaces_the_active_left_tab() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            panes: [
                PaneState {
                    tabs: vec!["/".into(), "/".into()],
                    active: 1,
                },
                PaneState::default(),
            ],
            active: 0,
            ..State::default()
        };
        let app = App::build(
            Core::default(),
            Config::default(),
            state,
            Some(tmp.path().into()),
            "/".into(),
        )
        .0;
        assert_eq!(
            cwds(&app, 0),
            [PathBuf::from("/"), tmp.path().canonicalize().unwrap()]
        );
    }

    #[test]
    fn argv_file_opens_its_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let app = App::build(
            Core::default(),
            Config::default(),
            State::default(),
            Some(file),
            "/".into(),
        )
        .0;
        assert_eq!(cwds(&app, 0), [tmp.path().canonicalize().unwrap()]);
    }

    #[test]
    fn regression_last_tab_home_dir_with_tilde_or_gone() {
        let config = Config {
            last_tab_close: LastTab::Home,
            home_dir: Some("~/shagoff-no-such-dir".into()),
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::CloseTab));
        let pending = app.panes[0]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone());
        assert_eq!(pending, Some(std::env::temp_dir())); // `~` = app.home, missing dir → parent
    }

    #[test]
    fn regression_drive_list_is_fixed_while_dialog_open() {
        let mut app = app_with(Config::default(), State::default());
        let drive = |label: &str, path: &str| Drive {
            label: label.into(),
            path: path.into(),
        };
        app.drives = vec![drive("/", "/"), drive("usr", "/usr"), drive("etc", "/etc")];
        let _ = app.update(Message::Key(Action::Drives(0))); // cursor on "/" (cwd is temp_dir)
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down)); // on "etc"
        app.drives.remove(1); // a stick was pulled: indices shift under the open dialog
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::Enter));
        let pending = app.panes[0]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone());
        assert_eq!(pending, Some(PathBuf::from("/etc")));
    }

    fn pending_path(app: &App) -> Option<PathBuf> {
        app.panes[0]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone())
    }

    #[test]
    fn regression_backspace_while_loading_goes_up_from_the_target() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.load(0, "/usr/share/doc".into(), None); // slow fs: not listed yet
        let _ = app.update(Message::Key(Action::Parent));
        assert_eq!(pending_path(&app), Some(PathBuf::from("/usr/share")));
        let _ = app.update(Message::Key(Action::Parent));
        assert_eq!(pending_path(&app), Some(PathBuf::from("/usr")));
    }

    #[test]
    fn enter_while_loading_is_ignored() {
        // The rows on screen belong to the dir being left; entering one would undo the navigation.
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(listed_ok(id, generation, &std::env::temp_dir())); // shows ".."
        let _ = app.load(0, "/usr".into(), None);
        let _ = app.update(Message::Key(Action::Enter)); // cursor on ".." of the old dir
        assert_eq!(pending_path(&app), Some(PathBuf::from("/usr")));
    }

    #[test]
    fn listing_error_names_the_dir_and_clears_on_next_key() {
        let mut app = app_with(Config::default(), State::default());
        let (id, generation) = {
            let _ = app.load(0, "/root/secret".into(), None);
            let t = app.panes[0].active();
            (t.id, t.pending.as_ref().unwrap().0)
        };
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: "/root/secret".into(),
            focus: None,
            result: Err("Permission denied".into()),
            space: None,
        });
        let err = app.panes[0].active().error.clone().unwrap_or_default();
        assert!(
            err.contains("/root/secret") && err.contains("Permission denied"),
            "{err}"
        );
        let _ = app.update(Message::Key(Action::Down));
        assert!(app.panes[0].active().error.is_none());
    }

    #[test]
    fn regression_f3_on_broken_symlink_reports_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("nowhere", tmp.path().join("dangling")).unwrap();
        let config = Config {
            viewer: vec!["/nonexistent/viewer".into()], // never launch anything from a test
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.load(0, tmp.path().into(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let entries = listing::scan(tmp.path(), false).unwrap();
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: tmp.path().into(),
            focus: Some("dangling".into()),
            result: Ok(entries),
            space: None,
        });
        let _ = app.update(Message::Key(Action::View));
        let err = app.panes[0].active().error.clone().unwrap_or_default();
        assert_eq!(err, fl!("broken-link", name = "dangling"));
    }

    /// Pane 0 listing 20 files, viewport `height` px tall, scrolled to the top.
    fn tall_list(height: f32) -> App {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let entries = (0..20).map(|i| entry(&format!("f{i:02}"))).collect();
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: std::env::temp_dir(),
            focus: None,
            result: Ok(entries),
            space: None,
        });
        let _ = app.update(Message::Resized(0, height));
        app
    }

    #[test]
    fn regression_click_on_half_visible_row_scrolls_it_in() {
        let mut app = tall_list(100.0); // rows 0..4 full, row 4 cut at 100 px
        let _ = app.update(Message::Click(0, 4));
        let t = app.panes[0].active();
        assert!(t.offset + t.height >= 5.0 * ROW_H, "offset {}", t.offset);
    }

    #[test]
    fn regression_shrinking_window_keeps_cursor_visible() {
        let mut app = tall_list(400.0);
        let _ = app.update(Message::Click(0, 10));
        let _ = app.update(Message::Resized(0, 100.0)); // window got smaller
        let t = app.panes[0].active();
        assert!(t.offset + t.height >= 11.0 * ROW_H, "offset {}", t.offset);
    }

    #[test]
    fn wheel_scroll_never_snaps_back_even_when_it_reports_a_smaller_height() {
        let mut app = tall_list(400.0);
        let _ = app.update(Message::Click(0, 10));
        let _ = app.update(Message::Scrolled(0, 0.0, 100.0));
        assert_eq!(app.panes[0].active().offset, 0.0);
    }

    #[test]
    fn resize_applies_to_every_tab_of_the_pane() {
        // One scrollable per pane: hidden tabs must not keep a stale height.
        let mut app = tall_list(400.0);
        let _ = app.update(Message::Key(Action::NewTab));
        let _ = app.update(Message::Resized(0, 150.0));
        assert!(app.panes[0].items().iter().all(|t| t.height == 150.0));
    }

    #[test]
    fn wheel_scroll_does_not_drag_the_view_back_to_the_cursor() {
        let mut app = tall_list(100.0);
        let _ = app.update(Message::Scrolled(0, 200.0, 100.0)); // user scrolled away
        assert_eq!(app.panes[0].active().offset, 200.0);
    }

    /// Pane 0 shows `names` (files) in temp_dir.
    fn files_app(names: &[&str]) -> App {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let entries = names.iter().map(|n| entry(n)).collect();
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: std::env::temp_dir(),
            focus: None,
            result: Ok(entries),
            space: None,
        });
        app
    }

    fn cursor_name(app: &App) -> String {
        app.panes[0].active().panel.current().unwrap().name.clone()
    }

    #[test]
    fn alt_letter_jumps_to_first_match() {
        let mut app = files_app(&["alpha", "beta", "bravo"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        assert_eq!(cursor_name(&app), "beta");
        assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("b"));
        let _ = app.update(Message::SearchInput("br".into()));
        assert_eq!(cursor_name(&app), "bravo");
    }

    #[test]
    fn alt_letter_without_match_opens_nothing() {
        let mut app = files_app(&["alpha"]);
        let _ = app.update(Message::Key(Action::QuickSearch('z')));
        assert!(app.search.is_none());
    }

    #[test]
    fn search_rejects_letter_without_match() {
        let mut app = files_app(&["alpha", "beta"]);
        let _ = app.update(Message::Key(Action::QuickSearch('a')));
        let _ = app.update(Message::SearchInput("az".into()));
        assert_eq!(app.search.as_ref().unwrap().text, "a");
        assert_eq!(cursor_name(&app), "alpha");
    }

    #[test]
    fn down_in_search_goes_to_next_match() {
        let mut app = files_app(&["b1", "x", "b2"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(cursor_name(&app), "b2");
        let _ = app.update(Message::Key(Action::Down)); // wraps
        assert_eq!(cursor_name(&app), "b1");
        assert!(app.search.is_some());
    }

    #[test]
    fn other_key_closes_search_and_acts() {
        let mut app = files_app(&["alpha", "beta"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        let _ = app.update(Message::Key(Action::Copy));
        assert!(app.search.is_none());
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Copy,
                ..
            })
        ));
    }

    #[test]
    fn enter_in_search_opens_the_dir() {
        let mut app = files_app(&[]);
        let t = app.panes[0].active();
        let (id, cwd) = (t.id, t.panel.cwd().to_path_buf());
        let _ = app.load(0, cwd.clone(), None);
        let generation = app.panes[0].active().pending.as_ref().unwrap().0;
        let mut sub = entry("sub");
        sub.kind = shagoff_core::listing::Kind::Dir;
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: cwd.clone(),
            focus: None,
            result: Ok(vec![sub]),
            space: None,
        });
        let _ = app.update(Message::Key(Action::QuickSearch('s')));
        let _ = app.update(Message::SearchSubmit);
        assert!(app.search.is_none());
        let pending = app.panes[0]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone());
        assert_eq!(pending, Some(cwd.join("sub")));
    }

    #[test]
    fn ctrl_s_filters_enter_keeps_escape_clears() {
        let mut app = files_app(&["a.rs", "b.md", "c.rs"]);
        let _ = app.update(Message::Key(Action::QuickFilter));
        let _ = app.update(Message::SearchInput("*.rs".into()));
        let shown = |app: &App| app.panes[0].active().panel.entries().len();
        assert_eq!(shown(&app), 3); // .., a.rs, c.rs
        let _ = app.update(Message::SearchSubmit);
        assert!(app.search.is_none());
        assert_eq!(app.panes[0].active().panel.filter(), Some("*.rs"));
        let _ = app.update(Message::DialogCancel); // Escape
        assert_eq!(app.panes[0].active().panel.filter(), None);
        assert_eq!(shown(&app), 4);
    }

    #[test]
    fn escape_closes_search_then_clears_filter() {
        let mut app = files_app(&["a.rs", "b.md"]);
        let _ = app.update(Message::Key(Action::QuickFilter));
        let _ = app.update(Message::SearchInput("a".into()));
        let _ = app.update(Message::DialogCancel); // in filter field: close + clear
        assert!(app.search.is_none());
        assert_eq!(app.panes[0].active().panel.filter(), None);

        let _ = app.update(Message::Key(Action::QuickSearch('a')));
        let _ = app.update(Message::DialogCancel); // in search field: only close
        assert!(app.search.is_none());
        assert_eq!(cursor_name(&app), "a.rs");
    }

    #[test]
    fn clicking_a_list_entry_opens_it() {
        let mut app = app_with(Config::default(), State::default());
        app.drives = vec![
            Drive {
                label: "/".into(),
                path: "/".into(),
            },
            Drive {
                label: "etc".into(),
                path: "/etc".into(),
            },
        ];
        let _ = app.update(Message::Key(Action::Drives(1)));
        app.panes[1].active_mut().pending = None;
        let _ = app.update(Message::ListPick(1));
        assert!(app.dialog.is_none());
        let pending = app.panes[1]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone());
        assert_eq!(pending, Some(PathBuf::from("/etc")));
    }

    /// Simulate a finished scan of `path` for the active tab of `side`.
    fn arrive(app: &mut App, side: usize, path: &Path) {
        let _ = app.load(side, path.into(), None);
        let t = app.panes[side].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: path.into(),
            focus: None,
            result: Ok(vec![]),
            space: None,
        });
    }

    fn pending_of(app: &App, side: usize) -> Option<PathBuf> {
        app.panes[side]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone())
    }

    #[test]
    fn alt_left_right_walk_tab_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/etc"));
        let _ = app.update(Message::Key(Action::HistoryBack));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/usr")));
        arrive(&mut app, 0, Path::new("/usr")); // the jump lands
        let _ = app.update(Message::Key(Action::HistoryForward));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
    }

    #[test]
    fn rescan_does_not_add_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, &std::env::temp_dir()); // the start dir's first listing
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/usr")); // Ctrl+R / watcher
        let _ = app.update(Message::Key(Action::HistoryBack));
        // One step back leaves /usr for the start dir, not /usr again.
        assert_eq!(pending_of(&app, 0), Some(std::env::temp_dir()));
    }

    #[test]
    fn alt_down_lists_history() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 0, Path::new("/etc"));
        let _ = app.update(Message::Key(Action::HistoryList));
        let Some(Dialog::List {
            kind: ListKind::History,
            items,
            ..
        }) = &app.dialog
        else {
            panic!("no history list");
        };
        assert_eq!(items[0].path, Path::new("/etc"));
        assert!(items.iter().any(|i| i.path == Path::new("/usr")));
    }

    #[test]
    fn ctrl_u_swaps_panes() {
        let mut app = app_with(Config::default(), State::default());
        arrive(&mut app, 0, Path::new("/usr"));
        arrive(&mut app, 1, Path::new("/etc"));
        app.space = [Some((1, 10)), Some((2, 20))];
        let _ = app.update(Message::Key(Action::SwapPanes));
        assert_eq!(app.space, [Some((2, 20)), Some((1, 10))]); // free space follows its pane
        assert_eq!(app.panes[0].active().panel.cwd(), Path::new("/etc"));
        assert_eq!(app.panes[1].active().panel.cwd(), Path::new("/usr"));
    }

    #[test]
    fn regression_swap_keeps_scan_in_flight() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.load(0, "/usr".into(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Key(Action::SwapPanes)); // tab now on side 1
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: "/usr".into(),
            focus: None,
            result: Ok(vec![]),
            space: None,
        });
        let t = app.panes[1].active();
        assert_eq!(t.panel.cwd(), Path::new("/usr"));
        assert!(t.pending.is_none(), "tab stuck in pending");
    }

    fn hotlist_items(app: &App) -> Vec<PathBuf> {
        match &app.dialog {
            Some(Dialog::List {
                kind: ListKind::Hotlist,
                items,
                ..
            }) => items.iter().map(|i| i.path.clone()).collect(),
            _ => panic!("hotlist not open"),
        }
    }

    #[test]
    fn hotlist_add_once_delete_clamps() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // row 0: add current dir
        let paths: Vec<_> = app.config.hotlist.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, std::slice::from_ref(&cwd));
        assert!(app.dialog.is_none());

        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // add again: no duplicate
        assert_eq!(app.config.hotlist.len(), 1);

        let _ = app.update(Message::Key(Action::Hotlist));
        assert_eq!(hotlist_items(&app).len(), 2); // add row + one entry
        let _ = app.update(Message::Key(Action::Delete)); // on the add row: nothing
        assert_eq!(app.config.hotlist.len(), 1);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Delete)); // remove the entry
        assert!(app.config.hotlist.is_empty());
        assert_eq!(hotlist_items(&app).len(), 1); // still open
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
    }

    #[test]
    fn hotlist_entry_opens_its_dir() {
        let config = Config {
            hotlist: vec![crate::config::HotEntry {
                name: "etc".into(),
                path: "/etc".into(),
            }],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Down));
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::Enter));
        assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
    }

    #[test]
    fn ctrl_h_flips_hidden_in_every_tab() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::NewTab));
        let _ = app.update(Message::Key(Action::ToggleHidden));
        assert!(app.config.show_hidden);
        for side in 0..2 {
            assert!(
                app.panes[side]
                    .items()
                    .iter()
                    .all(|t| t.panel.show_hidden())
            );
        }
    }

    #[test]
    fn ctrl_w_on_last_tab_obeys_config() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().pending = None;
        let before = cwds(&app, 0);
        let _ = app.update(Message::Key(Action::CloseTab));
        assert_eq!(cwds(&app, 0), before); // Nothing: no change
        assert!(app.panes[0].active().pending.is_none());

        let config = Config {
            last_tab_close: LastTab::Home,
            home_dir: Some("/".into()),
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::CloseTab));
        assert!(
            app.panes[0].active().pending.is_some(),
            "should load home_dir"
        );
    }

    #[test]
    fn config_change_of_show_hidden_applies_to_tabs() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Config(Config {
            show_hidden: true,
            ..Config::default()
        }));
        assert!(app.panes[1].active().panel.show_hidden());
    }

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

    fn press_with(key: Key, code: Code, mods: Modifiers) -> Option<Message> {
        let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers: mods,
            text: None,
            repeat: false,
        });
        route_event(
            event,
            event::Status::Captured,
            cosmic::iced::window::Id::unique(),
        )
    }

    #[test]
    fn regression_panel_keys_reach_the_app_while_a_field_has_focus() {
        // A focused text_input captures every key but Up/Down/Tab; F5 from quick search must still work.
        let f5 = press(Named::F5, Code::F5, event::Status::Captured);
        assert!(
            matches!(f5, Some(Message::FieldKey(Action::Copy))),
            "{f5:?}"
        );
        let pgdn = press(Named::PageDown, Code::PageDown, event::Status::Captured);
        assert!(
            matches!(pgdn, Some(Message::FieldKey(Action::PageDown))),
            "{pgdn:?}"
        );
        let ctrl_r = press_with(Key::Character("r".into()), Code::KeyR, Modifiers::CTRL);
        assert!(
            matches!(ctrl_r, Some(Message::FieldKey(Action::Reload))),
            "{ctrl_r:?}"
        );
    }

    #[test]
    fn text_editing_keys_stay_in_the_field() {
        for (named, code) in [
            (Named::Backspace, Code::Backspace),
            (Named::Delete, Code::Delete),
            (Named::Home, Code::Home),
            (Named::End, Code::End),
        ] {
            let msg = press(named, code, event::Status::Captured);
            assert!(msg.is_none(), "{named:?}: {msg:?}");
        }
        let space = press_with(Key::Character(" ".into()), Code::Space, Modifiers::empty());
        assert!(space.is_none(), "{space:?}");
        let ctrl_a = press_with(Key::Character("a".into()), Code::KeyA, Modifiers::CTRL);
        assert!(ctrl_a.is_none(), "{ctrl_a:?}");
    }

    #[test]
    fn field_key_acts_only_for_the_search_field() {
        let mut app = files_app(&["alpha", "beta"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        let _ = app.update(Message::FieldKey(Action::Copy));
        assert!(app.search.is_none());
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
        // Now a dialog text field has focus: its captured F-keys must not reach the panels.
        let _ = app.update(Message::FieldKey(Action::Delete));
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
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

    /// Point the active tab of `side` at `dir` and deliver its listing.
    fn listed_at(app: &mut App, side: usize, dir: &Path) {
        let _ = app.load(side, dir.into(), None);
        let t = app.panes[side].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: dir.into(),
            focus: None,
            result: Ok(listing::scan(dir, false).unwrap()),
            space: None,
        });
    }

    /// tmp/src/a (file) and tmp/dst/ (empty); pane 0 shows dst.
    fn paste_setup() -> (tempfile::TempDir, App, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a"), "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &dst);
        (tmp, app, src.join("a"))
    }

    #[test]
    fn paste_copy_starts_copy_into_active_cwd() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Copy));
    }

    #[test]
    fn paste_cut_starts_move() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    }

    #[test]
    fn paste_nothing_or_empty_does_nothing() {
        let (_tmp, mut app, _) = paste_setup();
        let _ = app.update(Message::Pasted(None));
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, Vec::new()))));
        assert!(app.job.is_none());
        assert!(app.panes[0].active().error.is_none());
    }

    #[test]
    fn paste_into_source_dir_reports_same_file() {
        let (_tmp, mut app, a) = paste_setup();
        listed_at(&mut app, 0, a.parent().unwrap());
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a.clone()]))));
        assert!(app.job.is_none());
        assert_eq!(
            app.panes[0].active().error.as_deref(),
            Some(fl!("plan-same-file", path = a.display().to_string()).as_str())
        );
    }

    #[test]
    fn paste_ignored_while_dialog_open() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Key(Action::Mkdir)); // opens the F7 dialog
        assert!(app.dialog.is_some());
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
        assert!(app.job.is_none());
    }

    #[test]
    fn regression_paste_during_navigation_lands_in_target() {
        let (tmp, mut app, a) = paste_setup();
        let next = tmp.path().join("next");
        std::fs::create_dir(&next).unwrap();
        let _ = app.load(0, next.clone(), None); // scan still pending: rows show dst
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
        assert!(app.job.is_some());
        let copied = next.join("a");
        for _ in 0..200 {
            if copied.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(copied.exists(), "pasted into the dir being left");
    }

    /// tmp/{a.txt, b.txt, c.txt}; pane 0 shows it with a.txt and b.txt marked (cursor on c.txt).
    fn mr_setup() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        for n in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.path().join(n), n).unwrap();
        }
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        for a in [Action::Down, Action::MarkDown, Action::MarkDown] {
            let _ = app.update(Message::Key(a));
        }
        let _ = app.update(Message::Key(Action::MultiRename));
        (tmp, app)
    }

    fn mr(app: &App) -> &dialogs::MultiRename {
        match &app.dialog {
            Some(Dialog::MultiRename(m)) => m,
            other => panic!("no multi-rename dialog: {:?}", other.is_some()),
        }
    }

    #[test]
    fn ctrl_m_opens_dialog_with_marked_files() {
        let (_tmp, app) = mr_setup();
        let m = mr(&app);
        let names: Vec<&str> = m.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
        assert!(m.taken.contains("c.txt") && !m.taken.contains("a.txt"));
        assert_eq!(m.current, None); // cursor is on c.txt, which is not renamed
    }

    #[test]
    fn mr_input_changes_preview() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
        let _ = app.update(Message::MrInput(dialogs::MrField::Digits, "2".into()));
        let rows = mr(&app).rows(&TimeZone::UTC);
        let new: Vec<&str> = rows.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(new, ["x01.txt", "x02.txt"]);
        let _ = app.update(Message::MrCase(Case::Upper));
        assert_eq!(mr(&app).rows(&TimeZone::UTC)[0].new, "X01.TXT");
    }

    #[test]
    fn mr_submit_with_problem_keeps_dialog() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "c".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(matches!(app.dialog, Some(Dialog::MultiRename(_))));
        assert!(app.job.is_none());
    }

    #[test]
    fn mr_submit_starts_move() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    }

    #[test]
    fn mr_unchanged_names_just_close() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none() && app.job.is_none());
    }

    #[test]
    fn mr_nothing_to_rename_opens_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path()); // only ".."
        let _ = app.update(Message::Key(Action::MultiRename));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn mr_skips_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(std::ffi::OsStr::from_bytes(b"bad\xff")), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // cursor on the bad name
        let _ = app.update(Message::Key(Action::MultiRename));
        assert!(app.dialog.is_none());
    }

    fn pack_setup() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("readme.txt"), "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &l);
        listed_at(&mut app, 1, &r);
        let _ = app.update(Message::Key(Action::Down)); // cursor on readme.txt
        (tmp, app)
    }

    #[test]
    fn alt_f5_opens_pack_dialog_with_default_name() {
        let (tmp, mut app) = pack_setup();
        let _ = app.update(Message::Key(Action::Pack));
        let Some(Dialog::Pack(p)) = &app.dialog else {
            panic!("no pack dialog")
        };
        assert_eq!(PathBuf::from(&p.path), tmp.path().join("r/readme.zip"));
        assert_eq!(p.format, Format::Zip);
    }

    #[test]
    fn pack_format_change_swaps_extension_and_is_remembered() {
        let (tmp, mut app) = pack_setup();
        let _ = app.update(Message::Key(Action::Pack));
        let _ = app.update(Message::PackFormat(Format::TarXz));
        let Some(Dialog::Pack(p)) = &app.dialog else {
            panic!()
        };
        assert_eq!(PathBuf::from(&p.path), tmp.path().join("r/readme.tar.xz"));
        assert_eq!(app.config.pack_format, "tar.xz");
    }

    #[test]
    fn pack_submit_starts_job() {
        let (_tmp, mut app) = pack_setup();
        let _ = app.update(Message::Key(Action::Pack));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Pack));
    }

    #[test]
    fn alt_f9_without_archives_does_nothing() {
        let (_tmp, mut app) = pack_setup();
        let _ = app.update(Message::Key(Action::Unpack));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn alt_f9_on_archive_opens_dialog_then_unpacks() {
        let (tmp, mut app) = pack_setup();
        std::fs::write(tmp.path().join("l/a.zip"), "").unwrap();
        listed_at(&mut app, 0, &tmp.path().join("l"));
        let _ = app.update(Message::Key(Action::Home));
        let _ = app.update(Message::Key(Action::Down)); // ".." → a.zip (sorted before readme.txt)
        let _ = app.update(Message::Key(Action::Unpack));
        assert!(
            matches!(&app.dialog, Some(Dialog::Unpack { archives, .. }) if archives.len() == 1)
        );
        let _ = app.update(Message::Toggle(dialogs::Toggle::OwnDir));
        assert!(matches!(
            &app.dialog,
            Some(Dialog::Unpack { own_dir: true, .. })
        ));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Unpack));
    }

    /// tmp/a.zip with d/f.txt and top.txt; pane 0 lists tmp (cursor on a.zip), pane 1 lists tmp/out.
    fn zip_setup() -> (tempfile::TempDir, App, PathBuf) {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
        for n in ["d/f.txt", "top.txt"] {
            z.start_file(n, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
        }
        z.finish().unwrap();
        std::fs::create_dir(tmp.path().join("out")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        listed_at(&mut app, 1, &tmp.path().join("out"));
        let _ = app.update(Message::Key(Action::End)); // "..", out/, a.zip
        (tmp, app, a)
    }

    #[test]
    fn enter_on_archive_goes_inside_and_back() {
        let (tmp, mut app, a) = zip_setup();
        let _ = app.update(Message::Key(Action::Enter));
        assert_eq!(app.panes[0].active().target(), a);
        listed_at(&mut app, 0, &a);
        let names: Vec<&str> = app.panes[0]
            .active()
            .panel
            .entries()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            names.contains(&"d") && names.contains(&"top.txt"),
            "{names:?}"
        );
        let _ = app.update(Message::Key(Action::Parent));
        assert_eq!(app.panes[0].active().target(), tmp.path());
    }

    #[test]
    fn f8_inside_archive_is_read_only() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Delete));
        assert!(app.dialog.is_none() && app.job.is_none());
        assert!(app.panes[0].active().error.is_some());
    }

    #[test]
    fn f5_inside_archive_starts_extract() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Copy));
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Extract));
    }

    #[test]
    fn f5_into_archive_panel_is_read_only() {
        let (tmp, mut app, a) = zip_setup();
        std::fs::write(tmp.path().join("out/x"), "1").unwrap();
        listed_at(&mut app, 0, &tmp.path().join("out"));
        listed_at(&mut app, 1, &a);
        let _ = app.update(Message::Key(Action::Down)); // x
        let _ = app.update(Message::Key(Action::Copy));
        assert!(app.dialog.is_none() && app.job.is_none());
        assert!(app.panes[0].active().error.is_some());
    }

    #[test]
    fn ctrl_c_inside_archive_extracts() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::ClipCopy));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Extract));
    }

    #[test]
    fn regression_pack_onto_existing_archive_is_not_read_only() {
        let (tmp, mut app, _a) = zip_setup();
        std::fs::write(tmp.path().join("out/note.txt"), "1").unwrap();
        std::fs::copy(tmp.path().join("a.zip"), tmp.path().join("out/note.zip")).unwrap();
        listed_at(&mut app, 0, &tmp.path().join("out"));
        listed_at(&mut app, 1, tmp.path());
        let _ = app.update(Message::Key(Action::End)); // note.zip, note.txt sorted: put cursor on note.txt
        let _ = app.update(Message::Key(Action::Home));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down)); // "..", note.txt, note.zip
        let _ = app.update(Message::Key(Action::Pack));
        let Some(Dialog::Pack(p)) = &mut app.dialog else {
            panic!("no pack dialog")
        };
        p.path = tmp.path().join("out/note.zip").display().to_string(); // an existing archive
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Pack));
    }

    #[test]
    fn regression_f5_right_after_entering_archive_copies_the_shown_rows() {
        let (tmp, mut app, a) = zip_setup();
        std::fs::write(tmp.path().join("plain.txt"), "1").unwrap();
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::End)); // plain.txt
        let _ = app.update(Message::Key(Action::Mark));
        let _ = app.load(0, a, None); // entering the archive, listing not back yet
        let _ = app.update(Message::Key(Action::Copy));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Copy));
    }

    fn find_dialog(app: &mut App) -> &mut dialogs::Find {
        match &mut app.dialog {
            Some(Dialog::Find(f)) => f,
            _ => panic!("no find dialog"),
        }
    }

    #[test]
    fn alt_f7_opens_find_with_panel_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let f = find_dialog(&mut app);
        assert_eq!(PathBuf::from(&f.dir), tmp.path());
        assert_eq!(f.mask, "*");
    }

    #[test]
    fn found_appends_results_and_stale_events_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let id = find_dialog(&mut app).id;
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(
            id,
            vec!["/a".into(), "/b".into()],
        )));
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(
            id + 1,
            vec!["/stale".into()],
        )));
        let f = find_dialog(&mut app);
        assert_eq!(f.results, [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(f.total, 2);
    }

    #[test]
    fn pick_goes_to_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/x.txt"), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let id = find_dialog(&mut app).id;
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(
            id,
            vec![tmp.path().join("sub/x.txt")],
        )));
        let _ = app.update(Message::Key(Action::Enter));
        assert!(app.dialog.is_none());
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
    }

    #[test]
    fn regression_enter_after_moving_into_results_goes_to_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let id = find_dialog(&mut app).id;
        let found = vec![tmp.path().join("a"), tmp.path().join("sub/b")];
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, found)));
        let _ = app.update(Message::Key(Action::Down));
        // The mask field still has focus: Enter arrives as its submit, not as a key.
        let _ = app.update(Message::FindSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
    }

    #[test]
    fn find_submit_after_typing_starts_a_search() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let id = find_dialog(&mut app).id;
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(
            id,
            vec![tmp.path().join("a")],
        )));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::FindInput(FindField::Mask, "*.rs".into()));
        let _ = app.update(Message::FindSubmit);
        let f = find_dialog(&mut app);
        assert!(f.stop.is_some() || f.results.is_empty()); // restarted
        assert_ne!(f.id, id);
    }

    #[test]
    fn find_settings_are_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindInput(FindField::Mask, "*.ini".into()));
        let _ = app.update(Message::FindInput(FindField::Text, "port".into()));
        let _ = app.update(Message::FindCase);
        let _ = app.update(Message::FindStart);
        let _ = app.update(Message::DialogCancel);
        assert_eq!(app.saved.find.mask, "*.ini"); // written to the state
        // A new run starts from the saved state.
        let mut app = app_with(Config::default(), app.saved.clone());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let f = find_dialog(&mut app);
        assert_eq!(
            (f.mask.as_str(), f.text.as_str(), f.case_sensitive),
            ("*.ini", "port", true)
        );
    }

    /// tmp/l: a (new), only_l; tmp/r: a (old), only_r. Pane 0 = l, pane 1 = r.
    fn sync_setup() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        for (p, t) in [
            (l.join("a"), None),
            (r.join("a"), Some(old)),
            (l.join("only_l"), None),
            (r.join("only_r"), None),
        ] {
            std::fs::write(&p, "x").unwrap();
            if let Some(t) = t {
                std::fs::File::options()
                    .write(true)
                    .open(&p)
                    .unwrap()
                    .set_modified(t)
                    .unwrap();
            }
        }
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &l);
        listed_at(&mut app, 1, &r);
        (tmp, app)
    }

    fn marked(app: &App, side: usize) -> Vec<String> {
        let p = &app.panes[side].active().panel;
        p.entries()
            .iter()
            .filter(|e| p.is_marked(e))
            .map(|e| e.name.clone())
            .collect()
    }

    #[test]
    fn shift_f2_marks_both_panels() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::CompareLists));
        assert_eq!(marked(&app, 0), ["a", "only_l"]);
        assert_eq!(marked(&app, 1), ["only_r"]);
    }

    fn sync_dlg(app: &mut App) -> &mut dialogs::SyncDlg {
        match &mut app.dialog {
            Some(Dialog::Sync(s)) => s,
            _ => panic!("no sync dialog"),
        }
    }

    fn compared(app: &mut App) {
        let s = sync_dlg(app);
        let o = s.options();
        let rows = shagoff_core::sync::compare(
            &s.left,
            &s.right,
            &o,
            &std::sync::atomic::AtomicBool::new(false),
        );
        let id = s.id;
        let _ = app.update(Message::SyncCompared(id, rows));
    }

    #[test]
    fn ctrl_shift_s_opens_sync_and_runs() {
        let (tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::SyncDirs));
        assert_eq!(sync_dlg(&mut app).left, tmp.path().join("l"));
        compared(&mut app);
        assert_eq!(sync_dlg(&mut app).rows.len(), 3);
        let _ = app.update(Message::SyncRun);
        assert!(app.dialog.is_none());
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Sync));
    }

    #[test]
    fn stale_compare_ignored_and_flip_cycles() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::SyncDirs));
        let id = sync_dlg(&mut app).id;
        let _ = app.update(Message::SyncCompared(id + 1, vec![]));
        assert!(sync_dlg(&mut app).running.is_some() || sync_dlg(&mut app).rows.is_empty());
        compared(&mut app);
        use shagoff_core::sync::Dir;
        assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToRight); // "a": left newer
        let _ = app.update(Message::SyncFlip(0));
        assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToLeft);
        let _ = app.update(Message::SyncFlip(0));
        assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::None);
        let _ = app.update(Message::SyncFlip(0));
        assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToRight);
    }

    #[test]
    fn sync_with_archive_panel_is_read_only() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 1, &a);
        let _ = app.update(Message::Key(Action::SyncDirs));
        assert!(app.dialog.is_none());
        assert!(app.panes[0].active().error.is_some());
    }

    #[test]
    fn shift_f2_through_the_event_route_marks() {
        let (_tmp, mut app) = sync_setup();
        let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::F2),
            modified_key: Key::Named(Named::F2),
            physical_key: Physical::Code(Code::F2),
            location: Location::Standard,
            modifiers: Modifiers::SHIFT,
            text: None,
            repeat: false,
        });
        let msg = route_event(
            event,
            event::Status::Ignored,
            cosmic::iced::window::Id::unique(),
        );
        assert!(
            matches!(msg, Some(Message::Key(Action::CompareLists))),
            "{msg:?}"
        );
        let _ = app.update(msg.unwrap());
        assert_eq!(marked(&app, 0), ["a", "only_l"]);
    }

    #[test]
    fn shift_f2_on_identical_dirs_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        for d in [&l, &r] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("f"), "x").unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
            std::fs::File::options()
                .write(true)
                .open(d.join("f"))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &l);
        listed_at(&mut app, 1, &r);
        let _ = app.update(Message::Key(Action::CompareLists));
        assert!(marked(&app, 0).is_empty() && marked(&app, 1).is_empty());
        assert!(app.panes[0].active().error.is_some()); // "identical" note in the status line
    }

    fn diff_dlg(app: &mut App) -> &mut dialogs::DiffDlg {
        match &mut app.dialog {
            Some(Dialog::Diff(d)) => d,
            _ => panic!("no diff dialog"),
        }
    }

    #[test]
    fn ctrl_shift_d_two_marked_in_one_panel() {
        let (tmp, mut app) = sync_setup(); // l: a, only_l
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::MarkDown));
        let _ = app.update(Message::Key(Action::MarkDown));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let d = diff_dlg(&mut app);
        assert_eq!(
            (d.left.clone(), d.right.clone()),
            (tmp.path().join("l/a"), tmp.path().join("l/only_l"))
        );
    }

    #[test]
    fn ctrl_shift_d_cursor_pair() {
        let (tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::Down)); // left: a
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down)); // right: a
        let _ = app.update(Message::Key(Action::CompareFiles));
        let d = diff_dlg(&mut app);
        assert_eq!(
            (d.left.clone(), d.right.clone()),
            (tmp.path().join("l/a"), tmp.path().join("r/a"))
        );
    }

    #[test]
    fn ctrl_shift_d_without_files_says_so() {
        let (_tmp, mut app) = sync_setup(); // cursors on ".."
        let _ = app.update(Message::Key(Action::CompareFiles));
        assert!(app.dialog.is_none());
        assert!(app.panes[0].active().error.is_some());
    }

    #[test]
    fn diff_next_prev_clamp_and_stale_ignored() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let id = diff_dlg(&mut app).id;
        let (rows, blocks) = shagoff_core::diff::rows("a\nb\nc\nd\n", "A\nb\nC\nd\n");
        let out = Arc::new(Ok(shagoff_core::diff::Outcome::Text { rows, blocks }));
        let _ = app.update(Message::DiffReady(id + 1, out.clone()));
        assert!(diff_dlg(&mut app).result.is_none());
        let _ = app.update(Message::DiffReady(id, out));
        let _ = app.update(Message::Key(Action::Up));
        assert_eq!(diff_dlg(&mut app).block, 0);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(diff_dlg(&mut app).block, 1); // two blocks: clamped
    }
}
