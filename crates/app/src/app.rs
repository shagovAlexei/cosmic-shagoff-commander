use crate::clip;
use crate::config::{self, Config, HotEntry, LastTab, State};
use crate::dialogs::{
    self, Dialog, FindField, InputOp, Item, ListItem, ListKind, MrField, SyncOpt, Toggle,
};
use crate::drawer::{self, Drawer, Setting, SettingsForm};
use crate::fl;
use crate::hotlist::{self, HotEdit, HotMsg};
use crate::jobs::{self, Job};
use crate::keymap::{self, Action, ListerKey};
use crate::lister::{self, Lister};
use cosmic::app::{Core, Task};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::keyboard::Modifiers;
use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
use cosmic::iced::{Size, Subscription, event, keyboard, mouse};
use cosmic::{Application, Element, widget};
use shagoff_core::archive::{self, Format};
use shagoff_core::clipboard::Kind as ClipKind;
use shagoff_core::cmdline::{self, Cmd};
use shagoff_core::drives::{self, Drive};
use shagoff_core::format::{self, TimeZone};
use shagoff_core::history::History;
use shagoff_core::launch;
use shagoff_core::listing::{self, Entry};
use shagoff_core::mask::Mask;
use shagoff_core::mount;
use shagoff_core::multirename::{self, Case, Rule};
use shagoff_core::ops::{self, ErrorChoice, Method, PlanError, Report, Resolution};
use shagoff_core::panel::{self, PARENT, Panel};
use shagoff_core::repack::Change;
use shagoff_core::session::{self, PaneState};
use shagoff_core::tabs::Tabs;
use shagoff_core::viewport;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";
// ponytail: list height is guessed until the scrollable reports its bounds (it does on the first event).
const FALLBACK_LIST_H: f32 = 400.0;
const FALLBACK_LIST_W: f32 = 500.0;

pub struct Flags {
    pub left: Option<PathBuf>,
}

pub struct Tab {
    /// Stable id: scan results are routed by it, so they land in the right tab even after switching.
    pub id: u64,
    pub panel: Panel,
    pub offset: f32,
    pub height: f32,
    pub width: f32,
    /// Ctrl+F1 Brief view (names in columns) instead of Full.
    pub brief: bool,
    /// Brief view: first column on screen.
    pub col: usize,
    /// Brief view: touchpad scroll not yet worth a column, px.
    wheel: f32,
    /// Generation and path of the scan in flight; any other result is stale.
    pub(crate) pending: Option<(u64, PathBuf)>,
    pub error: Option<String>,
    /// Alt+← / Alt+→ / Alt+↓.
    pub(crate) history: History,
    /// Alt+F7 "To panel": (dir searched, found paths) listed instead of the dir.
    pub(crate) results: Option<(PathBuf, Arc<Vec<PathBuf>>)>,
    /// TC locked tab: cannot be closed, leaving its dir opens a new tab.
    pub locked: bool,
    /// Own caption instead of the dir's name.
    pub name: Option<String>,
}

impl Tab {
    fn new(id: u64, cwd: PathBuf) -> Self {
        Self {
            id,
            panel: Panel::new(cwd),
            offset: 0.0,
            height: FALLBACK_LIST_H,
            width: FALLBACK_LIST_W,
            brief: false,
            col: 0,
            wheel: 0.0,
            pending: None,
            error: None,
            history: History::default(),
            results: None,
            locked: false,
            name: None,
        }
    }

    /// Tab caption: own name or the dir's, `*` in front when locked (as in TC).
    pub fn title(&self) -> String {
        let name = (self.name.clone()).unwrap_or_else(|| format::dir_title(self.panel.cwd()));
        if self.locked {
            format!("*{name}")
        } else {
            name
        }
    }

    /// Brief view: (rows per column, columns on screen) for rows `row_h` px high.
    pub fn brief_grid(&self, row_h: f32) -> (usize, usize) {
        (
            viewport::brief_rows(row_h, self.height),
            viewport::brief_cols(self.width),
        )
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
            width: self.width,
            brief: self.brief,
            col: self.col,
            wheel: 0.0,
            pending: None,
            error: None,
            history: self.history.clone(),
            results: self.results.clone(),
            locked: false,
            name: None,
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
    /// Rewriting an archive (a change inside one).
    Repack,
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
    /// Enter / F3 in an archive: what to do with the extracted file once the job succeeds.
    open: Option<(Open, PathBuf)>,
    /// "In background": no dialog, the panels work; progress shows above the status line.
    pub hidden: bool,
}

enum Open {
    Run(Vec<OsString>),
    /// F4 in an archive: run the editor, then watch the file to offer packing it back.
    Edit {
        argv: Vec<OsString>,
        archive: PathBuf,
        entry: PathBuf,
    },
    /// In the viewer, under this name.
    Lister(String),
}

/// What to do with a file extracted from an archive.
enum How {
    /// Program from the config (empty → the default).
    Run(Vec<String>, &'static [&'static str]),
    /// F4: the editor, and watch the file.
    Edit(Vec<String>),
    Lister,
}

/// A file extracted for F4 from an archive: a newer mtime offers to put it back.
pub struct Edited {
    pub file: PathBuf,
    pub archive: PathBuf,
    /// Its path inside the archive.
    pub entry: PathBuf,
    /// Last mtime seen (asked about or extracted).
    pub mtime: SystemTime,
}

/// Quick search (Alt+letter) or filter (Ctrl+S) field, shown in the pane's half of the status line.
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
    /// Help / about / settings in the side drawer.
    pub(crate) drawer: Option<Drawer>,
    /// Built once (the widget borrows it), rebuilt when the language changes.
    pub(crate) about: cosmic::widget::about::About,
    pub(crate) job: Option<Running>,
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
    /// Volumes not mounted yet (a stick just plugged in): listed with the drives, a pick mounts.
    pub volumes: Vec<mount::Volume>,
    /// When `volumes` was last asked for: gio runs at most every few seconds.
    volumes_at: Option<std::time::Instant>,
    /// (free, total) bytes of each pane's current disk.
    pub space: [Option<(u64, u64)>; 2],
    /// Status line: the last message (in the active pane's half).
    pub status: Option<Status>,
    /// User / group names for the status bar.
    pub owners: shagoff_core::owners::Owners,
    /// The last right press landed on a row (else below them): which context menu to show.
    pub ctx_entry: bool,
    /// A row took this right press: the list's own `RightEmpty`, published right after it for
    /// the same press (libcosmic does not capture right presses), must not undo that.
    ctx_row: bool,
    /// F3 viewer, shown in place of the panels.
    pub(crate) lister: Option<Box<Lister>>,
    /// Files from archives open in the editor (F4).
    pub(crate) edited: Vec<Edited>,
    /// Ctrl+F in progress: (address, cancel flag set by Esc).
    pub(crate) connecting: Option<(String, Arc<AtomicBool>)>,
    /// Command line under the panels (TC `path>`): its text, field id, history (last first).
    pub cmdline: String,
    pub(crate) cmd_id: widget::Id,
    pub commands: Vec<String>,
    /// Num+ / Num− masks, last first: the dialog opens with the last, ↑ / ↓ go through them.
    pub masks: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    /// Something runs in the background ("Mounting…"): kept until its result.
    Busy,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub kind: StatusKind,
    pub text: String,
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
    /// Context menu "Open": like a double click, not like the Enter key (which runs a typed command).
    OpenEntry,
    /// A context menu's popup surface (create / destroy), passed on to libcosmic.
    Surface(cosmic::surface::Action),
    /// Right press on row i: active pane, cursor there; the context menu opens on release.
    RightClick(usize, usize),
    /// Right press below the rows: the menu is about the dir itself.
    RightEmpty(usize),
    /// A button of one pane (column header, `\\`, `..`): that pane becomes active and acts.
    PaneKey(usize, Action),
    /// side, scroll offset y, viewport height (of the active tab)
    Scrolled(usize, f32, f32),
    /// side, real size of the pane's list (from a sensor: on_scroll misses resizes)
    Resized(usize, Size),
    /// side, mouse wheel over a Brief list: one column per notch
    BriefWheel(usize, mouse::ScrollDelta),
    SelectTab(usize, usize),
    CloseTabAt(usize, usize),
    Modifiers(Modifiers),
    DialogInput(String),
    DialogSubmit,
    DialogCancel,
    /// Escape in a window: the main one cancels the dialog; a popup (context menu) closes,
    /// since the menu widget only sees the main window's keys.
    Escape(cosmic::iced::window::Id),
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
    /// A show button (→ ← ≠ =): that kind of rows in or out of the list.
    SyncShow(shagoff_core::sync::Kind),
    SyncStop,
    /// Compare result for the sync dialog with this id.
    SyncCompared(u64, Vec<shagoff_core::sync::Row>),
    /// Cycle row i's arrow: → ← none.
    SyncFlip(usize),
    SyncRun,
    /// Sync dialog: the file mask as typed.
    SyncMask(String),
    /// Compare result for the diff dialog with this id.
    DiffReady(u64, Arc<Result<shagoff_core::diff::Outcome, String>>),
    DiffNext,
    DiffPrev,
    DiffScrolled(f32),
    DiffOpt(dialogs::DiffOpt),
    /// Copy the current block of differences left → right (true) or right → left.
    DiffCopy(bool),
    DiffSave,
    /// Alt+F7: the "regular expression" checkbox.
    FindRegex,
    FindNameRegex,
    FindArchives,
    /// Alt+Enter: toggle a permission bit / "also inside folders".
    PropsBit(u32),
    PropsRecursive,
    /// id, usage counted in the background (`None` = stopped).
    PropsUsage(u64, Option<shagoff_core::props::Usage>),
    /// Space on a dir counted it: (side, tab id, its parent, its name, bytes).
    DirSize(usize, u64, PathBuf, OsString, u64),
    /// chmod finished: (side, what failed).
    PropsDone(usize, Vec<String>),
    /// Alt+F7: the results into the dialog's panel (TC "Feed to listbox").
    FindFeed,
    Op(jobs::Event),
    Resolve(Resolution),
    ErrorAnswer(ErrorChoice),
    CancelJob,
    /// The progress dialog's "In background" / the status line's "Show".
    JobHide,
    JobShow,
    /// A panel key a focused text field captured (F-keys, PgUp/PgDn, Insert, Ctrl+…).
    FieldKey(Action),
    /// Click on entry i of the open list dialog.
    ListPick(usize),
    /// Hotlist settings (Ctrl+D → "Configure…").
    Hot(HotMsg),
    /// Text typed into the quick search / filter field.
    SearchInput(String),
    /// Files read from the system clipboard by Ctrl+V (`None`: no files there).
    Pasted(Option<(ClipKind, Vec<PathBuf>)>),
    /// Enter in that field.
    SearchSubmit,
    /// Command line: text edited, Enter (Shift+Enter: in a terminal), a character typed in the panel.
    CmdInput(String),
    CmdSubmit,
    /// `CmdSubmit` one step later: by then the Shift / Ctrl of that same Enter is in `mods`.
    CmdEnter,
    CmdType(char),
    /// The watched dir of this pane's active tab changed.
    Changed(usize),
    /// Drive button / drive list entry: (side, drive root). A path, not an index: the list can change.
    Drive(usize, PathBuf),
    CloseDrawer,
    Setting(Setting),
    /// Tab in a settings field.
    FocusNext,
    /// A link in the about drawer.
    OpenUrl(String),
    /// Ctrl+F dialog: the password field.
    ConnectPassword(String),
    /// Volumes from gio for the drive list of this side.
    Volumes(usize, Vec<mount::Volume>),
    /// gio's volumes for the drive list (the unmounted ones are kept).
    AllVolumes(Vec<mount::Volume>),
    /// A not-yet-mounted volume picked in the drive list: (side, device).
    MountVolume(usize, String),
    /// A volume or network location was mounted (for this side): its path.
    Mounted(usize, Result<PathBuf, mount::Error>),
    /// (unmounted drive root, result).
    Unmounted(PathBuf, Result<(), mount::Error>),
    Config(Config),
    /// Ctrl+F finished: (side, address, mount result).
    Connected(usize, String, Result<PathBuf, mount::Error>),
    /// Ctrl+F: a saved or found address into the field.
    ConnectPick(String),
    /// Ctrl+F: forget this saved address.
    ConnectForget(String),
    ConnectBrowse,
    /// What "Browse network" found.
    Browsed(Result<Vec<(String, String)>, mount::Error>),
    /// File read for the viewer with this id.
    ListerLoaded(u64, Arc<Result<lister::Loaded, String>>),
    ListerKey(ListerKey),
    /// A viewer letter key typed in the panel: the viewer's when it is open, else the command line's.
    Letter(ListerKey, char),
    /// x, y scroll offset, viewport height.
    ListerScrolled(f32, f32, f32),
    ListerResized(Size),
    /// Drop-down / A S K 8.
    ListerEncoding(shagoff_core::lister::Encoding),
    /// `W` / button.
    ListerWrap,
    ListerMode(shagoff_core::lister::Mode),
    /// F7 / the Find button: show the search field.
    ListerSearch,
    ListerQuery(String),
    /// Enter in the search field.
    ListerFind,
    /// Next (true) / previous file of the pane.
    ListerStep(bool),
    ListerClose,
    /// Every 2 s while files from archives are in the editor: look for saved changes.
    EditTick,
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
        // The language is chosen in main() from the system; a configured one replaces it.
        let theme = (cfg.app_theme != config::AppTheme::System)
            .then(|| cosmic::command::set_theme(cfg.app_theme.theme()));
        if !cfg.language.is_empty() {
            crate::i18n::init(&config::languages(&cfg.language, Vec::new()));
            core.window.header_title = fl!("app-title");
        }
        let (mut app, task) = Self::build(core, cfg, state, flags.left, home);
        let volumes = app.refresh_volumes(true);
        let task = Task::batch([task, volumes].into_iter().chain(theme));
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
        if !self.edited.is_empty() {
            subs.push(
                cosmic::iced::time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::EditTick),
            );
        }
        if self.job.is_none() {
            // Paused during file operations: finish_job rescans both panes anyway.
            for side in 0..2 {
                let cwd = self.panes[side].active().panel.cwd().to_path_buf();
                subs.push(crate::watcher::watch(side, cwd));
            }
        }
        Subscription::batch(subs)
    }

    /// The title bar's × and Alt+F4: the same temp cleanup as Exit (libcosmic exits after it).
    /// Only for the main window: a context menu's popup closes through here too.
    fn on_close_requested(&self, id: cosmic::iced::window::Id) -> Option<Message> {
        (id == self.window_id()).then_some(Message::Exit)
    }

    fn context_drawer(&self) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        self.drawer.as_ref().map(|d| drawer::view(self, d))
    }

    fn header_start(&self) -> Vec<Element<'_, Message>> {
        vec![crate::menu::bar(
            self.config.show_hidden,
            self.panes[self.active].active().locked,
        )]
    }

    fn view(&self) -> Element<'_, Message> {
        crate::view::view(self)
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        if let Some(d) = &self.dialog {
            return Some(dialogs::view(d, &self.input_id, &self.tz));
        }
        self.job
            .as_ref()
            .filter(|j| !j.hidden)
            .map(dialogs::progress)
    }

    fn footer(&self) -> Option<Element<'_, Message>> {
        Some(crate::view::footer(self))
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
            drawer: None,
            about: drawer::about(),
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
            volumes: Vec::new(),
            volumes_at: None,
            space: [None, None],
            status: None,
            owners: shagoff_core::owners::Owners::load(),
            ctx_entry: false,
            ctx_row: false,
            lister: None,
            edited: Vec::new(),
            connecting: None,
            cmdline: String::new(),
            cmd_id: widget::Id::unique(),
            commands: state.commands.clone(),
            masks: state.masks.clone(),
        };
        // A file opens its folder; a missing path keeps the saved tab.
        let left = left
            .and_then(|p| p.canonicalize().ok())
            .map(|p| session::existing_dir(&p, &home_fallback));
        for side in 0..2 {
            let (mut paths, active) = session::restore(&state.panes[side], &app.home);
            let saved = &state.panes[side];
            let locked = |i: usize| saved.locked.get(i).copied().unwrap_or(false);
            // A locked tab keeps its dir: the path opens in a new tab next to it.
            let extra = match &left {
                Some(l) if side == 0 && locked(active) => Some(l.clone()),
                Some(l) if side == 0 => {
                    paths[active] = l.clone();
                    None
                }
                _ => None,
            };
            let mut tabs = Tabs::new(app.new_tab(paths[0].clone()));
            for p in &paths[1..] {
                tabs.open_after(app.new_tab(p.clone()));
            }
            tabs.select(active);
            for (i, t) in tabs.items_mut().iter_mut().enumerate() {
                t.locked = locked(i);
                t.brief = saved.brief.get(i).copied().unwrap_or(false);
                t.name = saved.names.get(i).filter(|n| !n.is_empty()).cloned();
            }
            if let Some(l) = extra {
                tabs.open_after(app.new_tab(l));
            }
            app.panes[side] = tabs;
        }
        app.refresh_mounts();
        let task = app.load_all();
        (app, task)
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(action)
                if self.lister.is_some() && self.dialog.is_none() && !self.busy() =>
            {
                if let Some(task) = self.lister_action(action) {
                    return task;
                }
                return self.act(self.active, action);
            }
            Message::Key(action) => {
                if let Some(Dialog::List { kind, cursor, .. }) = &self.dialog
                    && *kind == ListKind::Hotlist
                    && *cursor < self.config.hotlist.len()
                    && matches!(action, Action::Delete | Action::DeletePermanent)
                {
                    let i = *cursor;
                    return self.hotlist_remove(i);
                }
                if let Some(Dialog::List { cursor, items, .. }) = &mut self.dialog {
                    // Separators are skipped, as in a menu.
                    let step = |from: usize, up: bool| {
                        let mut i = from;
                        loop {
                            i = match (up, i) {
                                (true, 0) => return from,
                                (true, _) => i - 1,
                                (false, _) if i + 1 >= items.len() => return from,
                                (false, _) => i + 1,
                            };
                            if items[i].kind != Item::Sep {
                                return i;
                            }
                        }
                    };
                    match action {
                        Action::Up => *cursor = step(*cursor, true),
                        Action::Down => *cursor = step(*cursor, false),
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
                            let shown = f.results.len().min(dialogs::FIND_SHOWN);
                            f.cursor = (f.cursor + 1).min(shown.saturating_sub(1));
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
                // The field lets ↑ / ↓ through as plain keys.
                if let Some(task) = self.mask_history(action) {
                    return task;
                }
                if let Some(d) = &self.dialog {
                    // Modal: panels must not move. Enter confirms a dialog without a text field.
                    if action == Action::Enter
                        && matches!(d, Dialog::ConfirmDelete { .. } | Dialog::Props(_))
                    {
                        return self.submit_dialog();
                    }
                    // Tab is not taken by text fields; walk the multi-rename form with it.
                    if action == Action::SwitchPane && matches!(d, Dialog::MultiRename(_)) {
                        return cosmic::iced::widget::operation::focus_next();
                    }
                    return Task::none();
                }
                if self.busy() {
                    return Task::none(); // panels wait for the running operation's dialog
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
                // Only the Enter key runs a typed line (the field lost focus to a click); a double
                // click or quick search's Enter opens the entry.
                if action == Action::Enter && self.cmd_ready() && !self.cmdline.trim().is_empty() {
                    return self.cmd_run(false);
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
            Message::RightClick(side, i) => {
                self.search = None;
                self.active = side;
                (self.ctx_entry, self.ctx_row) = (true, true);
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                // As in Explorer: the menu is about what was clicked. A marked row keeps the marks
                // (the menu is for all of them); an unmarked one drops them.
                let clicked_marked = t.panel.current().is_some_and(|e| t.panel.is_marked(e));
                if !clicked_marked {
                    t.panel.mark_all(false);
                }
                let tab = t.id;
                return self.reveal(side, tab);
            }
            Message::OpenEntry => return self.act(self.active, Action::Enter),
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(a),
                ));
            }
            Message::RightEmpty(side) => {
                self.search = None;
                self.active = side;
                // The same press on a row came first: that row's menu stands.
                if !std::mem::take(&mut self.ctx_row) {
                    self.ctx_entry = false;
                }
            }
            Message::DoubleClick(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].active_mut().panel.set_cursor(i);
                return self.act(side, Action::Enter);
            }
            Message::PaneKey(side, action) => {
                self.search = None;
                self.active = side;
                return self.act(side, action);
            }
            Message::Scrolled(side, offset, height) => {
                // Wheel / scrollbar: the view moves freely, the cursor stays where it is.
                let t = self.panes[side].active_mut();
                t.offset = offset;
                t.height = height;
            }
            Message::Resized(side, size) => {
                // One list widget per pane: every tab shares its viewport size.
                let t = self.panes[side].active();
                // Brief: any size change regroups the columns, the cursor may leave the screen.
                let shrunk = size.height < t.height || t.brief;
                for t in self.panes[side].items_mut() {
                    (t.width, t.height) = (size.width, size.height);
                }
                if shrunk {
                    let tab = self.panes[side].active().id;
                    return self.reveal(side, tab); // keep the cursor on screen
                }
            }
            Message::BriefWheel(side, delta) => {
                let row_h = self.row_h();
                let t = self.panes[side].active_mut();
                // Touchpads send many small pixel steps: one column per row height of travel.
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        t.wheel += y;
                        let steps = (t.wheel / row_h).trunc();
                        t.wheel -= steps * row_h;
                        steps
                    }
                };
                let (rows, cols) = t.brief_grid(row_h);
                let last = t.panel.entries().len().div_ceil(rows).saturating_sub(cols);
                t.col = if y > 0.0 {
                    t.col.saturating_sub(1)
                } else if y < 0.0 {
                    (t.col + 1).min(last)
                } else {
                    t.col
                };
            }
            Message::SelectTab(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].select(i);
                return self.tab_switched(side);
            }
            Message::CloseTabAt(side, i) => {
                self.search = None;
                if self.panes[side].items().get(i).is_some_and(|t| t.locked) {
                    self.say(StatusKind::Error, fl!("tab-locked"));
                    return Task::none();
                }
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
                        FindField::MinSize => &mut f.min_size,
                        FindField::MaxSize => &mut f.max_size,
                        FindField::Days => &mut f.days,
                    } = s;
                }
            }
            Message::FindCase => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.case_sensitive = !f.case_sensitive;
                    f.in_list = false;
                }
            }
            Message::FindRegex => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.regex = !f.regex;
                    f.in_list = false;
                }
            }
            Message::FindNameRegex => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.name_regex = !f.name_regex;
                    f.in_list = false;
                }
            }
            Message::FindArchives => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.archives = !f.archives;
                    f.in_list = false;
                }
            }
            Message::PropsBit(b) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog {
                    if p.mixed & !p.touched & b != 0 {
                        p.mode |= b; // `?` → on
                    } else {
                        p.mode ^= b;
                    }
                    p.touched |= b;
                }
            }
            Message::PropsRecursive => {
                if let Some(Dialog::Props(p)) = &mut self.dialog {
                    p.recursive = !p.recursive;
                    p.mixed = if p.recursive { 0o777 } else { p.own_mixed };
                }
            }
            Message::DirSize(side, tab, cwd, name, bytes) => {
                if let Some(t) = self.panes[side]
                    .items_mut()
                    .iter_mut()
                    .find(|t| t.id == tab)
                {
                    t.panel.set_dir_size(&cwd, name, bytes);
                }
            }
            Message::PropsUsage(id, usage) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && p.id == id
                {
                    p.usage = usage;
                }
            }
            Message::PropsDone(side, failed) => {
                match failed.first() {
                    None => self.say(StatusKind::Info, fl!("props-changed")),
                    Some(err) => {
                        let msg = fl!("props-failed", n = failed.len(), err = err.clone());
                        self.say(StatusKind::Error, msg);
                    }
                }
                // Both: the other panel may show the same dir.
                let tasks: Vec<_> = [side, 1 - side]
                    .into_iter()
                    .map(|s| {
                        let dir = self.panes[s].active().target();
                        self.reload(s, dir, None)
                    })
                    .collect();
                return Task::batch(tasks);
            }
            Message::FindFeed => return self.find_feed(),
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
                        SyncOpt::Mirror => &mut s.mirror,
                    };
                    *flag = !*flag;
                    // The rows must match the options they are synced with.
                    return self.start_compare();
                }
            }
            Message::SyncShow(kind) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && !s.hide.remove(&kind)
                {
                    s.hide.insert(kind);
                }
            }
            Message::SyncCompare => return self.start_compare(),
            Message::SyncMask(m) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog {
                    s.mask = m;
                    // The rows were for the old mask: nothing to run until compared again.
                    s.rows.clear();
                    s.confirm = false;
                }
            }
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
                    s.confirm = false; // asked again for the new rows
                }
            }
            Message::SyncFlip(i) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && let Some(row) = s.rows.get_mut(i)
                {
                    row.dir = shagoff_core::sync::next_dir(row);
                    s.confirm = false;
                }
            }
            Message::SyncRun => return self.start_sync(),
            Message::DiffReady(id, out) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.id == id
                {
                    if let Ok(shagoff_core::diff::Outcome::Text(t)) = out.as_ref() {
                        d.widths = dialogs::DiffDlg::measure(t);
                        if t.stamps.is_some() {
                            d.stamps = t.stamps; // read from disk now
                        }
                        // A copied block makes its side unsaved only if it changed the text.
                        if let Some((to_right, before)) = d.copying.take()
                            && t.src != before
                        {
                            if to_right {
                                d.dirty.1 = true;
                            } else {
                                d.dirty.0 = true;
                            }
                        }
                    }
                    d.result = Some(out);
                    // A re-compare (options, a copied block) stays near where it was.
                    d.block = d.block.min(d.blocks().len().saturating_sub(1));
                    return self.diff_scroll();
                }
            }
            Message::DiffScrolled(y) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    d.offset = y;
                }
            }
            Message::DiffOpt(o) => {
                // Not while a compare runs: it may hold a copied block this would drop.
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(shown) = d.result.clone().filter(|_| d.text().is_some())
                {
                    match o {
                        dialogs::DiffOpt::Space => d.opts.ignore_space ^= true,
                        dialogs::DiffOpt::Case => d.opts.ignore_case ^= true,
                    }
                    return self.start_diff_with(Some(shown), None);
                }
            }
            Message::DiffCopy(to_right) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                    && let Some(shown) = d.result.clone()
                {
                    d.copying = Some((to_right, src));
                    d.confirm_close = false;
                    let block = d.block;
                    return self.start_diff_with(Some(shown), Some((block, to_right)));
                }
            }
            Message::DiffSave => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                {
                    let mut failed = None;
                    let stamps = d.stamps.map_or([None, None], |s| s.map(Some));
                    let mut new = d.stamps;
                    for (i, (dirty, path, text)) in [
                        (&mut d.dirty.0, &d.left, &src.0),
                        (&mut d.dirty.1, &d.right, &src.1),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if *dirty {
                            match shagoff_core::diff::save(path, text, stamps[i]) {
                                Ok(()) => {
                                    *dirty = false;
                                    // Saved by us: the next save checks against this.
                                    if let (Some(n), Ok(m)) = (&mut new, std::fs::metadata(path)) {
                                        n[i] = shagoff_core::diff::stamp_of(&m);
                                    }
                                }
                                Err(e) => failed = Some(format!("{}: {e}", path.display())),
                            }
                        }
                    }
                    d.stamps = new;
                    d.confirm_close = false;
                    match failed {
                        Some(e) => self.say(StatusKind::Error, e),
                        None => self.say(StatusKind::Info, fl!("diff-saved")),
                    }
                }
            }
            Message::DiffNext => return self.diff_step(1),
            Message::DiffPrev => return self.diff_step(-1),
            Message::DialogSubmit => return self.submit_dialog(),
            Message::ListerLoaded(id, loaded) => {
                if let Some(l) = &mut self.lister
                    && l.id == id
                {
                    l.set_loaded(loaded);
                }
            }
            Message::ListerKey(k)
                if self.lister.is_some() && self.dialog.is_none() && !self.busy() =>
            {
                return match k {
                    ListerKey::Mode(m) => self.handle(Message::ListerMode(m)),
                    ListerKey::Next => self.lister_step(true),
                    ListerKey::Prev => self.lister_step(false),
                    ListerKey::Close => self.handle(Message::ListerClose),
                    ListerKey::FindPrev => self.lister_find(false, true),
                    ListerKey::Encoding(e) => self.handle(Message::ListerEncoding(e)),
                    ListerKey::Wrap => self.handle(Message::ListerWrap),
                };
            }
            Message::ListerKey(_) => {}
            Message::Letter(k, c) => {
                return self.handle(if self.lister.is_some() {
                    Message::ListerKey(k)
                } else {
                    Message::CmdType(c)
                });
            }
            Message::ListerScrolled(x, y, h) => {
                if let Some(l) = &mut self.lister {
                    (l.offset, l.height) = ((x, y), h);
                }
            }
            Message::ListerResized(size) => {
                if let Some(l) = &mut self.lister {
                    l.resized(size.width, size.height);
                }
            }
            Message::ListerEncoding(e) => {
                if let Some(l) = &mut self.lister {
                    l.set_encoding(e);
                    // From hex it is now text: a hex offset points nowhere in it.
                    return lister_scroll(l, 0.0, 0.0);
                }
            }
            Message::ListerWrap => {
                if let Some(l) = &mut self.lister {
                    l.toggle_wrap();
                    // The rows changed: the old offset points into other text.
                    return lister_scroll(l, 0.0, 0.0);
                }
            }
            Message::ListerMode(m) => {
                if let Some(l) = &mut self.lister {
                    let before = l.mode;
                    l.set_mode(m);
                    if l.mode != before {
                        return lister_scroll(l, 0.0, 0.0);
                    }
                }
            }
            Message::ListerSearch => return self.lister_search(),
            Message::ListerQuery(q) => {
                if let Some(l) = &mut self.lister {
                    l.query = q;
                }
            }
            Message::ListerFind => {
                if let Some(l) = &mut self.lister {
                    l.searching = false;
                }
                return self.lister_find(true, false);
            }
            Message::ListerStep(forward) => return self.lister_step(forward),
            Message::ListerClose => self.lister = None,
            Message::EditTick => {
                if self.dialog.is_some() || self.job.is_some() {
                    return Task::none(); // asked on a later tick
                }
                // Gone (temp cleaned, file deleted): stop watching it.
                self.edited
                    .retain(|e| std::fs::symlink_metadata(&e.file).is_ok());
                for e in &mut self.edited {
                    if let Ok(m) = std::fs::symlink_metadata(&e.file).and_then(|m| m.modified())
                        && m != e.mtime
                    {
                        e.mtime = m;
                        self.dialog = Some(Dialog::UpdateArchive {
                            file: e.file.clone(),
                            archive: e.archive.clone(),
                            entry: e.entry.clone(),
                        });
                        break;
                    }
                }
            }
            // Unsaved copied blocks: the first Esc / Cancel only warns.
            Message::DialogCancel
                if matches!(&self.dialog, Some(Dialog::Diff(d))
                    if (d.dirty.0 || d.dirty.1) && !d.confirm_close) =>
            {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    d.confirm_close = true;
                }
            }
            Message::Escape(window) if window != self.window_id() => {
                return self.update(Message::Surface(cosmic::surface::action::destroy_popup(
                    window,
                )));
            }
            Message::Escape(_) => return self.update(Message::DialogCancel),
            Message::DialogCancel => match self.dialog.take() {
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(Resolution::Cancel);
                }
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(ErrorChoice::Cancel);
                }
                Some(Dialog::Connect { .. }) => {
                    if let Some((_, cancel)) = &self.connecting {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Some(_) => {}
                None => {
                    if self.drawer.is_some() {
                        self.set_drawer(None);
                    } else if let Some(s) = self.search.take() {
                        if s.filter {
                            self.panes[s.side].active_mut().panel.set_filter(None);
                        }
                    } else if self.busy() {
                        self.cancel_job(); // not one in the background: Esc is for the panels
                    } else if let Some(l) = &mut self.lister {
                        if l.searching {
                            l.searching = false;
                        } else {
                            self.lister = None;
                        }
                    } else if let Some((_, cancel)) = &self.connecting {
                        cancel.store(true, Ordering::Relaxed);
                    } else if self.config.show_cmdline && !self.cmdline.is_empty() {
                        self.cmdline.clear();
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
            Message::JobHide | Message::JobShow => {
                if let Some(j) = &mut self.job {
                    j.hidden = matches!(message, Message::JobHide);
                }
            }
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
            Message::Hot(m) => {
                if let Some(Dialog::Hotlist(h)) = &mut self.dialog {
                    let cwd = self.panes[h.side].active().panel.cwd().to_path_buf();
                    h.update(m, &cwd);
                }
            }
            // Quick search, viewer search or command line; not a dialog's or a settings field.
            Message::FieldKey(action) => {
                if let Some(task) = self.mask_history(action) {
                    return task;
                }
                if self.dialog.is_none() && self.drawer.is_none() {
                    return self.handle(Message::Key(action));
                }
                // Tab is ours (pane switch), so the settings fields get it here: next field.
                if action == Action::SwitchPane && self.dialog.is_none() {
                    return cosmic::iced::runtime::widget::operation::focus_next();
                }
            }
            Message::CmdInput(text) => {
                self.cmdline = text;
                // Erased: the panel gets its keys back (Backspace goes up again).
                if self.cmdline.is_empty() {
                    return unfocus();
                }
            }
            // The field's Enter comes before the modifier change of the same key batch: wait a
            // step, or Shift+Enter ran without the terminal (the field's message is handled first).
            Message::CmdSubmit => return cosmic::task::message(Message::CmdEnter),
            // The field takes Ctrl+Enter as Enter: here it inserts the name / path under the cursor.
            Message::CmdEnter if self.mods.control() => {
                let action = if self.mods.shift() {
                    Action::CmdPath
                } else {
                    Action::CmdName
                };
                return self
                    .cmd_action(self.active, action)
                    .unwrap_or_else(Task::none);
            }
            Message::CmdEnter => return self.cmd_run(self.mods.shift()),
            Message::CmdType(c) => {
                if self.cmd_ready() {
                    self.cmdline.push(c);
                    return self.cmd_focus();
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
                    return self.reload(side, cwd, None);
                }
            }
            Message::Drive(side, path) => {
                self.search = None;
                if !self.busy() {
                    self.dialog = None;
                    return self.go_to(side, path);
                }
            }
            Message::ConnectPassword(s) => {
                if let Some(Dialog::Connect { password, .. }) = &mut self.dialog {
                    *password = s;
                }
            }
            Message::AllVolumes(vols) => {
                self.volumes = vols.into_iter().filter(|v| v.mount.is_none()).collect();
            }
            Message::MountVolume(side, device) => {
                self.search = None;
                if self.busy() {
                    return Task::none();
                }
                self.dialog = None;
                self.say(StatusKind::Busy, fl!("mounting"));
                return blocking(
                    move || mount::mount_device(&device),
                    move |r| Message::Mounted(side, r),
                );
            }
            Message::Volumes(side, vols) => {
                if let Some(Dialog::List {
                    kind: ListKind::Drives,
                    side: s,
                    items,
                    ..
                }) = &mut self.dialog
                    && *s == side
                {
                    let fresh: Vec<_> = vols
                        .into_iter()
                        .filter(|v| v.mount.is_none())
                        .filter(|v| {
                            !items
                                .iter()
                                .any(|i| i.path.as_os_str() == v.device.as_str())
                        })
                        .collect();
                    items.extend(fresh.into_iter().map(|v| ListItem {
                        label: v.name,
                        path: v.device.into(),
                        kind: Item::Mount,
                    }));
                }
            }
            Message::Mounted(side, result) => {
                self.refresh_mounts();
                let volumes = self.refresh_volumes(true);
                match result {
                    Ok(path) => {
                        self.clear_busy();
                        return Task::batch([self.go_to(side, path), volumes]);
                    }
                    Err(mount::Error::Cancelled) => {
                        self.status = None; // the Busy "Connecting…"
                        self.say(StatusKind::Info, fl!("connect-cancelled"));
                    }
                    Err(e) => self.say(StatusKind::Error, mount_error(&e)),
                }
                return volumes;
            }
            // Ctrl+F's result: remembered on success, then as any mount.
            Message::Connected(side, url, result) => {
                if self.connecting.as_ref().is_some_and(|(u, _)| *u == url) {
                    self.connecting = None;
                }
                if result.is_ok() {
                    let list = mount::remember(&self.config.connections, &url);
                    self.save_connections(list);
                    if matches!(self.dialog, Some(Dialog::Connect { .. })) {
                        self.dialog = None;
                    }
                } else if let Some(Dialog::Connect { note, .. }) = &mut self.dialog
                    && let Err(e) = &result
                    && *e != mount::Error::Cancelled
                {
                    *note = Some((true, mount_error(e)));
                    self.status = None; // the Busy "Connecting…"
                    return Task::none();
                }
                return self.handle(Message::Mounted(side, result));
            }
            Message::ConnectPick(u) => {
                if let Some(Dialog::Connect { url, .. }) = &mut self.dialog {
                    *url = u;
                }
                return widget::text_input::focus(self.input_id.clone());
            }
            // By value: the config may have changed since the dialog opened.
            Message::ConnectForget(url) => {
                let mut list = self.config.connections.clone();
                list.retain(|u| *u != url);
                if let Some(Dialog::Connect { saved, .. }) = &mut self.dialog {
                    saved.clone_from(&list);
                }
                self.save_connections(list);
            }
            Message::ConnectBrowse => {
                if let Some(Dialog::Connect { browsing, .. }) = &mut self.dialog {
                    *browsing = true;
                    return blocking(mount::browse, Message::Browsed);
                }
            }
            Message::Browsed(r) => {
                if let Some(Dialog::Connect {
                    found,
                    browsing,
                    note,
                    ..
                }) = &mut self.dialog
                {
                    *browsing = false;
                    *note = match r {
                        Ok(list) if list.is_empty() => Some((false, fl!("connect-none-found"))),
                        Ok(list) => {
                            *found = list;
                            None
                        }
                        Err(e) => Some((true, mount_error(&e))),
                    };
                }
            }
            Message::Unmounted(root, result) => {
                self.refresh_mounts();
                self.volumes_at = None; // the next listing asks gio again
                if let Err(e) = result {
                    self.say(StatusKind::Error, mount_error(&e));
                    return Task::none();
                }
                self.clear_busy();
                // ponytail: only the active tab of each pane moves; other tabs inside show an error.
                let inside: Vec<usize> = (0..2)
                    .filter(|&s| self.panes[s].active().target().starts_with(&root))
                    .collect();
                let home = self.home.clone();
                return Task::batch(inside.into_iter().map(|s| self.load(s, home.clone(), None)));
            }
            Message::Config(c) => return self.apply_config(c),
            Message::CloseDrawer => self.set_drawer(None),
            Message::FocusNext => return cosmic::iced::widget::operation::focus_next(),
            Message::Setting(s) => return self.set(s),
            Message::OpenUrl(url) => {
                if let Err(e) = spawn_detached(&["xdg-open".into(), url.into()]) {
                    log::warn!("xdg-open: {e}");
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
                locked: self.panes[s].items().iter().map(|t| t.locked).collect(),
                brief: self.panes[s].items().iter().map(|t| t.brief).collect(),
                names: (self.panes[s].items().iter())
                    .map(|t| t.name.clone().unwrap_or_default())
                    .collect(),
            }),
            active: self.active,
            find: self.find.clone(),
            commands: self.commands.clone(),
            masks: self.masks.clone(),
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

    /// Parent window of context menu popups.
    pub fn window_id(&self) -> cosmic::iced::window::Id {
        self.core
            .main_window_id()
            .unwrap_or(cosmic::iced::window::Id::RESERVED)
    }

    /// File row height of the current skin; every scroll computation uses it.
    pub fn row_h(&self) -> f32 {
        self.config.skin.row_h()
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Scan `path` for the active tab of `side` in the background; the result lands in `Message::Listed`.
    /// Go to `path` in the active tab (leaves search results).
    fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
        // A locked tab keeps its dir: going elsewhere happens in a new tab next to it (TC).
        if path != self.panes[side].active().panel.cwd() {
            self.leave_locked(side);
        }
        self.panes[side].active_mut().results = None;
        let volumes = self.refresh_volumes(false);
        Task::batch([self.reload(side, path, focus), volumes])
    }

    /// Ask gio for unmounted volumes (`force`: now; else not within 3 s of the last time).
    fn refresh_volumes(&mut self, force: bool) -> Task<Message> {
        let now = std::time::Instant::now();
        let recent = self
            .volumes_at
            .is_some_and(|t| now - t < std::time::Duration::from_secs(3));
        if recent && !force {
            return Task::none();
        }
        self.volumes_at = Some(now);
        blocking(mount::list, |r| Message::AllVolumes(r.unwrap_or_default()))
    }

    /// A locked tab keeps its dir: what would move it happens in an unlocked copy opened next to it.
    fn leave_locked(&mut self, side: usize) {
        if self.panes[side].active().locked {
            let id = self.next_id();
            let copy = self.panes[side].active().duplicate(id);
            self.panes[side].open_after(copy);
        }
    }

    /// Read the active tab's dir again (search results stay: they are re-checked).
    fn reload(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
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
        // Going anywhere else leaves the search results.
        if t.results.as_ref().is_some_and(|(root, _)| *root != path) {
            t.results = None;
        }
        let results = t.results.as_ref().map(|(_, r)| r.clone());
        let tab = t.id;
        let show_hidden = t.panel.show_hidden();
        Task::perform(
            async move {
                let p = path.clone();
                let (result, space) = tokio::task::spawn_blocking(move || {
                    let result = read_listing(&p, show_hidden, results.as_deref());
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
        // A message lasts until the next action; "working…" until its result.
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.kind != StatusKind::Busy)
        {
            self.status = None;
        }
        // Names there are paths: Ctrl+M would move files between dirs, Shift+F2 compares by name.
        if matches!(action, Action::MultiRename | Action::CompareLists)
            && (0..2).any(|s| self.panes[s].active().results.is_some())
        {
            self.say(StatusKind::Error, fl!("results-unsupported"));
            return Task::none();
        }
        if let Some(task) = self.cmd_action(side, action) {
            return task;
        }
        // Backspace / ".." in search results: back to the dir that was searched.
        let t = self.panes[side].active_mut();
        let on_parent = t.panel.current().is_some_and(|e| e.name == PARENT);
        if (action == Action::Parent || (action == Action::Enter && on_parent))
            && let Some((root, _)) = t.results.take()
        {
            return self.load(side, root, None);
        }
        match action {
            Action::QuickSearch(c) => return self.quick_search(side, c),
            Action::QuickFilter => return self.quick_filter(side),
            Action::Disconnect => return self.disconnect(side),
            Action::Help | Action::About | Action::Settings | Action::Donate => {
                self.toggle_drawer(action);
                return Task::none();
            }
            Action::CopyNames | Action::CopyPaths => {
                let paths = self.panes[side].active().panel.targets();
                if paths.is_empty() {
                    return Task::none();
                }
                let text = shagoff_core::clipboard::names_text(&paths, action == Action::CopyPaths);
                let (n, one) = (paths.len(), text.clone());
                let msg = if action == Action::CopyPaths {
                    fl!("copied-paths", n = n, text = one)
                } else {
                    fl!("copied-names", n = n, text = one)
                };
                self.say(StatusKind::Info, msg);
                return cosmic::iced::clipboard::write(text);
            }
            _ => {}
        }
        if self.read_only(side, action) {
            self.say(StatusKind::Error, fl!("archive-read-only"));
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
            if matches!(d, Dialog::Props(_)) {
                self.dialog = Some(d);
                return self.start_usage();
            }
            if let Dialog::List {
                kind: ListKind::Drives,
                side: s,
                ..
            } = d
            {
                self.dialog = Some(d);
                return list_volumes(s);
            }
            let focus = matches!(
                d,
                Dialog::Mask { .. }
                    | Dialog::Input { .. }
                    | Dialog::MultiRename(_)
                    | Dialog::Pack(_)
                    | Dialog::Unpack { .. }
                    | Dialog::Find(_)
                    | Dialog::Connect { .. }
            );
            self.dialog = Some(d);
            return if focus {
                widget::text_input::focus(self.input_id.clone())
            } else {
                Task::none()
            };
        }
        let row_h = self.row_h();
        let t = self.panes[side].active_mut();
        let (rows, cols) = t.brief_grid(row_h);
        let (brief, rows) = (t.brief, rows as isize);
        let page = if brief {
            rows * cols as isize
        } else {
            viewport::page_rows(row_h, t.height) as isize
        };
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
            Action::Left if brief => panel.move_cursor(-rows),
            Action::Right if brief => panel.move_cursor(rows),
            Action::Left | Action::Right => {}
            Action::ViewBrief | Action::ViewFull => {
                let t = self.panes[side].active_mut();
                t.brief = action == Action::ViewBrief;
                return Task::batch([self.reveal(side, tab), self.restore_scroll(side)]);
            }
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
                                How::Run(Vec::new(), &["xdg-open"]),
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
                    self.say(StatusKind::Error, fl!("open-failed", err = err.to_string()));
                }
            }
            // From where the tab is going, so quick Backspaces on a slow fs are not lost.
            Action::Parent => {
                if let Some((path, focus)) = panel::parent_of(&target) {
                    return self.load(side, path, Some(focus));
                }
            }
            Action::Root => return self.load(side, "/".into(), None),
            Action::Mark => {
                panel.toggle_mark();
                // TC: Space on a dir also counts its size (not in archives: not real dirs).
                let dir = panel
                    .current()
                    .filter(|e| e.is_dir() && e.name != PARENT && panel.dir_size(e).is_none())
                    .map(|e| (panel.cwd().to_path_buf(), e.os_name.clone()));
                if let Some((cwd, name)) =
                    dir.filter(|(cwd, n)| !loading && !inside_archive(&cwd.join(n)))
                {
                    let count = count_dirs(side, tab, cwd, vec![name]);
                    return Task::batch([count, self.reveal(side, tab)]);
                }
            }
            // TC Alt+Shift+Enter: the size of every dir here, without marking.
            Action::CountDirs => {
                let cwd = panel.cwd().to_path_buf();
                if loading || inside_archive(&cwd.join("x")) {
                    return Task::none();
                }
                let names = panel
                    .entries()
                    .iter()
                    .filter(|e| e.is_dir() && e.name != PARENT && panel.dir_size(e).is_none())
                    .map(|e| e.os_name.clone())
                    .collect();
                return count_dirs(side, tab, cwd, names);
            }
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
            Action::View if self.config.internal_viewer => return self.view_current(side),
            Action::View | Action::Edit => {
                let cwd = panel.cwd().to_path_buf();
                let current = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| e.os_name.clone());
                if let (Some(os_name), Some((arc, inner))) = (current, archive::split_path(&cwd)) {
                    // A link entry would open (and on "update", overwrite) whatever it points to.
                    let link = panel.current().is_some_and(|e| e.is_link);
                    if action == Action::Edit && link {
                        self.say(StatusKind::Error, fl!("archive-edit-link"));
                        return Task::none();
                    }
                    let how = if action == Action::Edit {
                        How::Edit(self.config.editor.clone())
                    } else {
                        How::Run(self.config.viewer.clone(), &["xdg-open"])
                    };
                    return self.open_from_archive(side, arc, inner, os_name, how);
                }
                let t = self.panes[side].active_mut();
                let panel = &t.panel;
                let file = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| (panel.cwd().join(&e.os_name), e.name.clone()));
                if let Some((file, name)) = file {
                    if !file.exists() {
                        self.say(StatusKind::Error, fl!("broken-link", name = name));
                        return Task::none();
                    }
                    let argv = if action == Action::View {
                        launch::command(&self.config.viewer, &["xdg-open"], &file)
                    } else {
                        launch::command(&self.config.editor, &["cosmic-edit"], &file)
                    };
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(StatusKind::Error, fl!("open-failed", err = err.to_string()));
                    }
                }
            }
            // Opened by `dialog_for` above.
            Action::Drives(_) => {}
            Action::QuickSearch(_)
            | Action::QuickFilter
            | Action::Disconnect
            | Action::Help
            | Action::About
            | Action::Settings
            | Action::Donate
            | Action::CopyNames
            | Action::CopyPaths
            | Action::CmdName
            | Action::CmdPath
            | Action::CmdCwd
            | Action::CmdPrevious
            | Action::CmdHistory
            | Action::TabRename => {} // handled above
            Action::Connect => {} // always a dialog
            Action::HistoryBack | Action::HistoryForward => {
                let step = |h: &mut History| {
                    if action == Action::HistoryBack {
                        h.back()
                    } else {
                        h.forward()
                    }
                };
                // Stepped in the copy a locked tab opens, so the locked one keeps its place.
                if step(&mut self.panes[side].active().history.clone()).is_some() {
                    self.leave_locked(side);
                    if let Some(path) = step(&mut self.panes[side].active_mut().history) {
                        return self.load(side, path, None);
                    }
                }
            }
            // Reached only when `dialog_for` found no pair of files.
            Action::CompareFiles => {
                let in_archive =
                    |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
                let msg = if in_archive(side) || in_archive(1 - side) {
                    fl!("diff-in-archive")
                } else {
                    fl!("diff-pick-two")
                };
                self.say(StatusKind::Error, msg);
            }
            Action::CompareLists => {
                let (l, r) = shagoff_core::sync::compare_lists(
                    self.panes[0].active().panel.entries(),
                    self.panes[1].active().panel.entries(),
                );
                // Nothing to mark must not look like a key that did nothing.
                if l.is_empty() && r.is_empty() {
                    self.say(StatusKind::Info, fl!("compare-identical"));
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
                return self.start_extract(side, &cwd, &paths, dest, false);
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
            | Action::CopySame
            | Action::Properties
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
            // Also the drive list: a stick plugged in shows up (TC rereads drives too).
            Action::Reload => {
                let cwd = panel.cwd().to_path_buf();
                self.refresh_mounts();
                let volumes = self.refresh_volumes(true);
                return Task::batch([self.reload(side, cwd, None), volumes]);
            }
            Action::TabOpen | Action::TabOpenOther => {
                let path = self.tab_target(side);
                let to = if action == Action::TabOpen {
                    side
                } else {
                    1 - side
                };
                let tab = self.new_tab(path.clone());
                self.panes[to].open_after(tab);
                return self.load(to, path, None);
            }
            Action::TabCopyOther | Action::TabMoveOther => {
                let tab = if action == Action::TabCopyOther {
                    let id = self.next_id();
                    Some(self.panes[side].active().duplicate(id))
                } else {
                    let i = self.panes[side].active_index();
                    self.panes[side].take(i)
                };
                let Some(tab) = tab else {
                    self.say(StatusKind::Error, fl!("tab-last"));
                    return Task::none();
                };
                self.panes[1 - side].open_after(tab);
                self.active = 1 - side;
                let mut tasks = vec![self.tab_switched(1 - side)];
                if action == Action::TabMoveOther {
                    tasks.push(self.tab_switched(side));
                }
                return Task::batch(tasks);
            }
            Action::TabLock => {
                let t = self.panes[side].active_mut();
                t.locked = !t.locked;
                // A scan into another dir would still land and move it.
                if t.locked && t.target() != t.panel.cwd() {
                    t.pending = None;
                }
                return Task::none();
            }
            Action::CloseOtherTabs => {
                self.panes[side].retain(|t| t.locked);
                return Task::none();
            }
            Action::NewTab => {
                let id = self.next_id();
                let copy = self.panes[side].active().duplicate(id);
                self.panes[side].open_after(copy);
                return self.restore_scroll(side);
            }
            Action::CloseTab => {
                if self.panes[side].active().locked {
                    self.say(StatusKind::Error, fl!("tab-locked"));
                    return Task::none();
                }
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
                input: self.masks.first().cloned().unwrap_or_else(|| "*".into()),
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
            Action::Properties => {
                let paths = panel.targets();
                (!paths.is_empty()).then(|| self.props_dialog(side, paths))
            }
            Action::CopySame => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                let path = panel.cwd().join(&e.os_name);
                // In search results the shown name is a path; offer the file's own name.
                let name = path
                    .file_name()
                    .map_or(e.name.clone(), |n| n.to_string_lossy().into_owned());
                let parent = path.parent()?.to_path_buf();
                Some(input(InputOp::Copy, vec![path], {
                    // Relative to the panel's dir: in search results that is not the file's dir.
                    if parent == panel.cwd() {
                        name
                    } else {
                        parent.join(name).to_string_lossy().into_owned()
                    }
                }))
            }
            Action::Mkdir => Some(input(InputOp::Mkdir, Vec::new(), String::new())),
            // The caption as shown; left as the dir's name it keeps following the dir.
            Action::TabRename => {
                let t = self.panes[side].active();
                let shown = t.name.clone();
                let shown = shown.unwrap_or_else(|| format::dir_title(t.panel.cwd()));
                Some(input(InputOp::TabName, Vec::new(), shown))
            }
            Action::Rename => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                let path = panel.cwd().join(&e.os_name);
                // In search results the shown name is a path; offer the file's own name.
                let name = path
                    .file_name()
                    .map_or(e.name.clone(), |n| n.to_string_lossy().into_owned());
                Some(input(InputOp::Rename, vec![path], name))
            }
            Action::Delete | Action::DeletePermanent => {
                let paths = panel.targets();
                // An archive has no trash: deleting from one is always for good (TC too).
                let in_archive = archive::split_path(panel.cwd()).is_some();
                (!paths.is_empty()).then_some(Dialog::ConfirmDelete {
                    side,
                    permanent: action == Action::DeletePermanent || in_archive,
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
                        kind: Item::Dir,
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
                        kind: Item::Dir,
                    })
                    .collect(),
            }),
            Action::Connect => Some(Dialog::Connect {
                side,
                url: "sftp://".into(),
                password: String::new(),
                saved: self.config.connections.clone(),
                found: Vec::new(),
                browsing: false,
                note: None,
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
                    regex: self.find.regex,
                    name_regex: self.find.name_regex,
                    archives: self.find.archives,
                    min_size: self.find.min_size.clone(),
                    max_size: self.find.max_size.clone(),
                    days: self.find.days.clone(),
                    error: None,
                    root: None,
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
                    offset: 0.0,
                    widths: (420.0, 420.0),
                    opts: Default::default(),
                    dirty: (false, false),
                    confirm_close: false,
                    stamps: None,
                    copying: None,
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
                // As before: equal files hidden until asked for.
                hide: HashSet::from([shagoff_core::sync::Kind::Same]),
                mirror: false,
                mask: String::new(),
                confirm: false,
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
            Dialog::Props(p) => {
                // Only the touched bits, as shown now: entries keep the rest of their own modes.
                let (set, clear) = (p.mode & p.touched, !p.mode & p.touched & 0o7777);
                if p.touched == 0 {
                    return Task::none();
                }
                let (side, paths, recursive) = (p.side, p.paths.clone(), p.recursive);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            shagoff_core::props::chmod(&paths, set, clear, recursive)
                                .into_iter()
                                .map(|(p, e)| format!("{}: {e}", p.display()))
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |failed| cosmic::Action::App(Message::PropsDone(side, failed)),
                )
            }
            Dialog::Mask {
                side,
                select,
                input,
            } => {
                self.masks = cmdline::remember(&self.masks, &input);
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
                op: InputOp::TabName,
                side,
                input,
                ..
            } => {
                let name = input.trim();
                let t = self.panes[side].active_mut();
                let own = !name.is_empty() && name != format::dir_title(t.panel.cwd());
                t.name = own.then(|| name.to_string());
                Task::none()
            }
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
                    self.say(StatusKind::Error, fl!("archive-read-only"));
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
            Dialog::Connect {
                side,
                url,
                password,
                saved,
                found,
                browsing,
                note,
            } => {
                let address = url.trim().to_string();
                // Stays open while connecting: the error lands in it, Cancel stops it.
                let busy = self.connecting.is_some();
                self.dialog = Some(Dialog::Connect {
                    side,
                    url,
                    password: password.clone(),
                    saved,
                    found,
                    browsing,
                    note: if busy || address.is_empty() {
                        note
                    } else {
                        Some((false, fl!("connecting")))
                    },
                });
                if busy || address.is_empty() {
                    return Task::none();
                }
                let url = address;
                let cancel = Arc::new(AtomicBool::new(false));
                self.connecting = Some((url.clone(), cancel.clone()));
                self.say(StatusKind::Busy, fl!("connecting"));
                let address = url.clone();
                blocking(
                    move || mount::connect(&url, &password, &cancel),
                    move |r| Message::Connected(side, address, r),
                )
            }
            Dialog::Unpack {
                side,
                archives,
                path,
                own_dir,
            } => {
                let dest = self.panes[side].active().target().join(path.trim());
                if into_archive(&dest) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
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
            Dialog::Hotlist(h) => {
                self.save_hotlist(h.list);
                Task::none()
            }
            d @ Dialog::List { .. } => {
                let i = match &d {
                    Dialog::List { cursor, .. } => *cursor,
                    _ => 0,
                };
                self.dialog = Some(d);
                self.pick(i)
            }
            Dialog::UpdateArchive {
                file,
                archive,
                entry,
            } => {
                let job = Job::Repack {
                    archive,
                    change: Change::Replace { entry, file },
                    move_sources: false,
                };
                self.start_job(self.active, OpKind::Repack, job, None)
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
        let first = || match Path::new(name).components().next() {
            Some(Component::Normal(first)) => Some(first.to_string_lossy().into_owned()),
            _ => None,
        };
        if let Some((archive, inner)) = archive::split_path(&cwd) {
            if Path::new(name).is_absolute() {
                self.say(StatusKind::Error, plan_error(&PlanError::BadName));
                return Task::none();
            }
            let job = Job::Repack {
                archive,
                change: Change::Mkdir(inner.join(name)),
                move_sources: false,
            };
            return self.start_job(side, OpKind::Repack, job, first());
        }
        match std::fs::create_dir_all(cwd.join(name)) {
            Ok(()) => self.reload(side, cwd, first()),
            Err(e) => {
                self.say(
                    StatusKind::Error,
                    fl!("mkdir-failed", path = name, err = e.to_string()),
                );
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
        // From the rows' own dir: right after Enter on an archive they still show the dir left.
        let from = sources
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let from_archive = from.as_deref().and_then(archive::split_path);
        let target = cwd.join(session::expand_home(Path::new(input), &self.home));
        let to_archive = (op != InputOp::Rename)
            .then(|| archive::split_path(&target))
            .flatten();
        match (from_archive, to_archive) {
            (Some(_), Some(_)) => {
                self.say(StatusKind::Error, fl!("archive-read-only"));
                return Task::none();
            }
            (Some((archive, inner)), None) if op == InputOp::Rename => {
                let Some(name) = sources.first().and_then(|p| p.file_name()) else {
                    return Task::none();
                };
                if input.is_empty() || Path::new(name) == Path::new(input) {
                    return Task::none();
                }
                // As outside archives (`rename_pairs`): a new name, not a path.
                if input.contains('/') || input == "." || input == ".." {
                    self.say(StatusKind::Error, plan_error(&PlanError::BadName));
                    return Task::none();
                }
                let job = Job::Repack {
                    archive,
                    change: Change::Rename {
                        from: inner.join(name),
                        to: inner.join(input),
                    },
                    move_sources: false,
                };
                return self.start_job(side, OpKind::Repack, job, Some(input.to_string()));
            }
            (Some(_), None) => {
                let from = from.unwrap_or_default();
                return self.start_extract(side, &from, &sources, target, op == InputOp::Move);
            }
            (None, Some((archive, inner))) => {
                // The archive itself (or a dir holding it) among the sources: F6 would delete it.
                let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                let arc = real(&archive);
                if let Some(s) = sources.iter().find(|s| arc.starts_with(real(s))) {
                    let e = PlanError::IntoItself(s.clone());
                    self.say(StatusKind::Error, plan_error(&e));
                    return Task::none();
                }
                let job = Job::Repack {
                    archive,
                    change: Change::Add { sources, inner },
                    move_sources: op == InputOp::Move,
                };
                return self.start_job(side, OpKind::Repack, job, None);
            }
            (None, None) => {}
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
            (ops::plan(&sources, &target), None)
        };
        let pairs = match planned {
            Ok(pairs) if pairs.is_empty() => return Task::none(), // rename to the same name
            Ok(pairs) => pairs,
            Err(e) => {
                self.say(StatusKind::Error, plan_error(&e));
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
        // Paths through an archive are not real files: extract first (F5).
        if (0..2).any(|s| archive::split_path(&self.panes[s].active().target()).is_some()) {
            return None;
        }
        let panel = &self.panes[side].active().panel;
        let marked: Vec<&Entry> = panel
            .entries()
            .iter()
            .filter(|e| panel.is_marked(e))
            .collect();
        match marked.as_slice() {
            [] => Some((file(0)?, file(1)?)),
            [a, b] if !a.is_dir() && !b.is_dir() => {
                Some((panel.cwd().join(&a.os_name), panel.cwd().join(&b.os_name)))
            }
            // Something is marked, but not two files: don't silently compare other ones.
            _ => None,
        }
    }

    fn start_diff(&mut self) -> Task<Message> {
        self.start_diff_with(None, None)
    }

    /// Compare the dialog's files: read them (`src` None), or the texts in hand with new options
    /// or after copying block `copy.0` (`copy.1`: left → right).
    fn start_diff_with(
        &mut self,
        shown: Option<Arc<Result<shagoff_core::diff::Outcome, String>>>,
        copy: Option<(usize, bool)>,
    ) -> Task<Message> {
        use shagoff_core::diff;
        let id = self.next_id();
        let Some(Dialog::Diff(d)) = &mut self.dialog else {
            return Task::none();
        };
        d.id = id;
        // Nothing to save or copy from until this result is in (it may hold a copied block).
        d.result = None;
        let (a, b, o) = (d.left.clone(), d.right.clone(), d.opts);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let Some(Ok(diff::Outcome::Text(t))) = shown.as_deref() else {
                        return diff::compare_with(&a, &b, o).map_err(|e| e.to_string());
                    };
                    let (mut l, mut r) = (t.src.0.clone(), t.src.1.clone());
                    if let Some((block, to_right)) = copy
                        && let Some(new) = diff::copy_block(t, block, to_right)
                    {
                        if to_right {
                            r = new;
                        } else {
                            l = new;
                        }
                    }
                    Ok(diff::Outcome::Text(diff::rows_with(
                        &l,
                        &r,
                        diff::ROWS_LIMIT,
                        o,
                    )))
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
        let Some(Dialog::Sync(s)) = &mut self.dialog else {
            return Task::none();
        };
        if s.running.is_some() {
            return Task::none();
        }
        let p = shagoff_core::sync::plan(&s.left, &s.right, &s.rows);
        if p.to_right.is_empty() && p.to_left.is_empty() && p.delete.is_empty() {
            return Task::none();
        }
        // Deleting: the button turns into "delete and synchronize" first.
        if !p.delete.is_empty() && !s.confirm {
            s.confirm = true;
            return Task::none();
        }
        let side = s.side;
        self.dialog = None;
        let job = Job::Sync {
            to_right: p.to_right,
            to_left: p.to_left,
            delete: p.delete,
        };
        self.start_job(side, OpKind::Sync, job, None)
    }

    /// Alt+Enter: facts about the selection and its bits (of the first entry that has any).
    fn props_dialog(&self, side: usize, paths: Vec<PathBuf>) -> Dialog {
        use std::os::unix::fs::MetadataExt;
        let mut facts = Vec::new();
        let metas: Vec<_> = paths
            .iter()
            .map(|p| std::fs::symlink_metadata(p).ok())
            .collect();
        if let [path] = paths.as_slice() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            facts.push((fl!("props-name"), name.into_owned()));
            if let Some(m) = &metas[0] {
                let kind = if m.is_dir() {
                    fl!("props-folder")
                } else if m.file_type().is_symlink() {
                    let to = std::fs::read_link(path).unwrap_or_default();
                    format!("{} {}", fl!("props-link"), to.display())
                } else {
                    let ext = path.extension().unwrap_or_default().to_string_lossy();
                    format::mime_type(&ext).unwrap_or_else(|| "—".into())
                };
                facts.push((fl!("props-type"), kind));
                let mtime = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                facts.push((fl!("props-modified"), format::date(mtime, &self.tz)));
                let owner = self.owners.name(m.uid(), m.gid());
                facts.push((fl!("props-owner-line"), owner));
            }
        } else {
            facts.push((fl!("props-selected"), fl!("props-items", n = paths.len())));
        }
        let modes: Vec<u32> = paths
            .iter()
            .filter_map(|p| shagoff_core::props::mode(p))
            .collect();
        let all = modes.iter().fold(0o7777, |a, m| a & m);
        let any = modes.iter().fold(0, |a, m| a | m);
        let has_dir = metas.iter().flatten().any(|m| m.is_dir());
        Dialog::Props(Box::new(dialogs::Props {
            side,
            paths,
            facts,
            mode: if modes.is_empty() { 0 } else { all },
            mixed: any & !all,
            own_mixed: any & !all,
            touched: 0,
            has_dir,
            recursive: false,
            usage: None,
            id: 0, // set by `start_usage`
            stop: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// Count the open properties dialog's selection in the background.
    fn start_usage(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Props(p)) = &mut self.dialog else {
            return Task::none();
        };
        p.id = id;
        let (paths, stop) = (p.paths.clone(), p.stop.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || shagoff_core::props::usage(&paths, &stop))
                    .await
                    .unwrap_or_default()
            },
            move |u| cosmic::Action::App(Message::PropsUsage(id, u)),
        )
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
            regex: f.regex,
            name_regex: f.name_regex,
            archives: f.archives,
            min_size: f.min_size.clone(),
            max_size: f.max_size.clone(),
            days: f.days.clone(),
        };
        let root = self.panes[side].active().target().join(f.dir.trim());
        let root_searched = root.clone();
        let text = f.text.trim().to_string();
        let q = match find_query(f, text) {
            Ok(q) => shagoff_core::search::Query {
                hidden: self.panes[side].active().panel.show_hidden(),
                ..q
            },
            Err(e) => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.error = Some(e);
                }
                return Task::none();
            }
        };
        let (stop, events) = crate::find::spawn(id, root, q);
        if let Some(Dialog::Find(f)) = &mut self.dialog {
            if let Some(old) = f.stop.replace(stop) {
                old.store(true, Ordering::Relaxed);
            }
            (f.id, f.cursor, f.total, f.in_list) = (id, 0, 0, false);
            f.error = None;
            f.root = Some(root_searched);
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
                f.results.extend(paths);
            }
            FindEvent::Dir(id, d) if id == f.id => f.current = d.display().to_string(),
            FindEvent::Done(id) if id == f.id => {
                f.stop = None;
                f.current.clear();
            }
            _ => {} // a previous search
        }
    }

    /// "To panel": the active tab of the dialog's panel lists the results until it is left.
    fn find_feed(&mut self) -> Task<Message> {
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let side = f.side;
        // The dir that was searched, not the field as edited since.
        let Some(root) = f.root.clone() else {
            return Task::none();
        };
        let paths = Arc::new(f.results.clone());
        // Only real files can be listed and operated on; say what was left out.
        let in_archives = paths.iter().filter(|p| inside_archive(p)).count();
        self.dialog = None; // stops the search
        if in_archives > 0 {
            self.say(StatusKind::Info, fl!("find-feed-archives", n = in_archives));
        }
        self.active = side;
        self.leave_locked(side);
        self.panes[side].active_mut().results = Some((root.clone(), paths));
        self.reload(side, root, None)
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
        // F6: delete from the archive what came out.
        move_after: bool,
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
        if move_after {
            let job = Job::ExtractMove {
                archive,
                inner,
                names,
                dest,
            };
            return self.start_job(side, OpKind::Move, job, None);
        }
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
        how: How,
    ) -> Task<Message> {
        let dir = match archive::fresh_temp_dir(&archive::temp_root()) {
            Ok(d) => d,
            Err(e) => {
                self.say(StatusKind::Error, fl!("open-failed", err = e.to_string()));
                return Task::none();
            }
        };
        let file = dir.join(&name);
        let open = match how {
            How::Run(prog, default) => Open::Run(launch::command(&prog, default, &file)),
            How::Edit(prog) => Open::Edit {
                argv: launch::command(&prog, &["cosmic-edit"], &file),
                archive: archive.clone(),
                entry: inner.join(&name),
            },
            How::Lister => Open::Lister(name.to_string_lossy().into_owned()),
        };
        let job = Job::Extract {
            archive,
            inner,
            names: vec![PathBuf::from(name)],
            dest: dir,
        };
        let task = self.start_job(side, OpKind::Extract, job, None);
        if let Some(j) = &mut self.job {
            j.open = Some((open, file));
        }
        task
    }

    /// F3 with the built-in viewer: the file under the cursor (extracted first inside an archive).
    fn view_current(&mut self, side: usize) -> Task<Message> {
        let panel = &self.panes[side].active().panel;
        let cwd = panel.cwd().to_path_buf();
        let Some(e) = panel.current().filter(|e| !e.is_dir() && e.name != PARENT) else {
            return Task::none();
        };
        let (os_name, name) = (e.os_name.clone(), e.name.clone());
        if let Some((arc, inner)) = archive::split_path(&cwd) {
            return self.open_from_archive(side, arc, inner, os_name, How::Lister);
        }
        let file = cwd.join(&os_name);
        if !file.exists() {
            self.say(StatusKind::Error, fl!("broken-link", name = name));
            return Task::none();
        }
        self.open_lister(side, file, name)
    }

    fn open_lister(&mut self, side: usize, file: PathBuf, name: String) -> Task<Message> {
        let id = self.next_id();
        match &mut self.lister {
            Some(l) => l.reopen(name.clone(), id),
            None => self.lister = Some(Box::new(Lister::new(side, name.clone(), id))),
        }
        self.search = None;
        let read = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lister::Loaded::read(&file, &name))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerLoaded(id, Arc::new(r))),
        );
        let top = self
            .lister
            .as_deref_mut()
            .map_or(Task::none(), |l| lister_scroll(l, 0.0, 0.0));
        Task::batch([read, top])
    }

    /// Keys while the viewer is open; `None`: not the viewer's (help, settings…), act as usual.
    fn lister_action(&mut self, action: Action) -> Option<Task<Message>> {
        if matches!(action, Action::Left | Action::Right) {
            self.lister.as_ref()?;
            return Some(self.lister_scroll_x(action == Action::Right));
        }
        let l = self.lister.as_deref_mut()?;
        Some(match action {
            Action::Help | Action::About | Action::Settings | Action::Donate => return None,
            Action::View => self.lister_find(true, true),
            Action::Mkdir => self.lister_search(),
            a => match l.scroll_y(a) {
                Some(y) => lister_scroll(l, l.offset.0, y),
                None => Task::none(),
            },
        })
    }

    fn lister_search(&mut self) -> Task<Message> {
        let Some(l) = &mut self.lister else {
            return Task::none();
        };
        l.searching = true;
        widget::text_input::focus(l.input.clone())
    }

    /// Enter in the field (`again` false) or F3 / Shift+F3; no text yet → open the field.
    fn lister_find(&mut self, forward: bool, again: bool) -> Task<Message> {
        let Some(l) = &mut self.lister else {
            return Task::none();
        };
        if l.query.is_empty() {
            return self.lister_search();
        }
        match l.find(forward, again) {
            Some(y) => lister_scroll(l, 0.0, y),
            None => {
                let q = l.query.clone();
                self.say(StatusKind::Info, fl!("lister-not-found", query = q));
                Task::none()
            }
        }
    }

    fn lister_scroll_x(&mut self, right: bool) -> Task<Message> {
        self.lister.as_deref_mut().map_or(Task::none(), |l| {
            let (x, y) = (l.scroll_x(right), l.offset.1);
            lister_scroll(l, x, y)
        })
    }

    /// N / P: the next / previous file of the pane (dirs skipped), cursor moved there too.
    fn lister_step(&mut self, forward: bool) -> Task<Message> {
        let Some(side) = self.lister.as_ref().map(|l| l.side) else {
            return Task::none();
        };
        let t = self.panes[side].active_mut();
        let entries = t.panel.entries();
        let file = |i: &usize| !entries[*i].is_dir() && entries[*i].name != PARENT;
        let cur = t.panel.cursor();
        let next = if forward {
            (cur + 1..entries.len()).find(file)
        } else {
            (0..cur).rev().find(file)
        };
        let Some(i) = next else {
            return Task::none();
        };
        t.panel.set_cursor(i);
        let tab = t.id;
        let shown = self.lister.as_ref().map(|l| l.id);
        let view = self.view_current(side);
        if self.job.is_none() && self.lister.as_ref().map(|l| l.id) == shown {
            // Not opened (broken link): the cursor stays with the file on screen.
            self.panes[side].active_mut().panel.set_cursor(cur);
            return view;
        }
        Task::batch([self.reveal(side, tab), view])
    }

    /// Actions that would change an archive (this panel inside one, or the other one as target).
    fn read_only(&self, side: usize, action: Action) -> bool {
        let inside = |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
        // F4, F7, F8, Shift+F6 inside and F5 / F6 into or out of one rewrite the archive.
        match action {
            // Shift+F5 inside one copies from the archive into itself.
            Action::MultiRename
            | Action::ClipCut
            | Action::ClipPaste
            | Action::CopySame
            | Action::Properties => inside(side),
            Action::Pack | Action::Unpack | Action::SyncDirs => inside(side) || inside(1 - side),
            Action::Copy | Action::Move | Action::ClipCopy => inside(side) && inside(1 - side),
            _ => false,
        }
    }

    fn start_delete(&mut self, side: usize, permanent: bool, paths: Vec<PathBuf>) -> Task<Message> {
        let dir = paths
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        if let Some((archive, _)) = dir.and_then(|d| archive::split_path(&d)) {
            let entries = paths
                .iter()
                .filter_map(|p| p.strip_prefix(&archive).ok().map(Path::to_path_buf))
                .collect();
            let job = Job::Repack {
                archive,
                change: Change::Delete(entries),
                move_sources: false,
            };
            return self.start_job(side, OpKind::Repack, job, None);
        }
        self.start_job(side, OpKind::Delete, Job::Delete { paths, permanent }, None)
    }

    fn start_job(
        &mut self,
        side: usize,
        kind: OpKind,
        job: Job,
        focus: Option<String>,
    ) -> Task<Message> {
        // One at a time: a second one waits for the first (in the background) to end.
        if self.job.is_some() {
            self.say(StatusKind::Error, fl!("job-running"));
            return Task::none();
        }
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
            hidden: false,
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
                let name = ops::unique_name(&dst.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.dialog = Some(Dialog::Conflict {
                    src,
                    dst,
                    reply,
                    name,
                });
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
        let mut view = Task::none();
        if let Some((open, file)) = job.open
            && !report.cancelled
            && file.exists()
        {
            match open {
                Open::Run(argv) => {
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(StatusKind::Error, fl!("open-failed", err = err.to_string()));
                    }
                }
                Open::Edit {
                    argv,
                    archive,
                    entry,
                } => match spawn_detached(&argv) {
                    Ok(()) => {
                        let mtime = std::fs::symlink_metadata(&file)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        self.edited.push(Edited {
                            file,
                            archive,
                            entry,
                            mtime,
                        });
                    }
                    Err(err) => {
                        self.say(StatusKind::Error, fl!("open-failed", err = err.to_string()))
                    }
                },
                Open::Lister(name) => view = self.open_lister(side, file, name),
            }
        }
        self.panes[side]
            .active_mut()
            .panel
            .unmark(&report.completed);
        let here = self.panes[side].active().panel.cwd().to_path_buf();
        let there = self.panes[1 - side].active().panel.cwd().to_path_buf();
        Task::batch([
            self.reload(side, here, job.focus),
            self.reload(1 - side, there, None),
            view,
        ])
    }

    /// Keep tab `tab`'s cursor row fully visible. An inactive tab only gets its offset updated;
    /// `restore_scroll` applies it when that tab is shown.
    fn reveal(&mut self, side: usize, tab: u64) -> Task<Message> {
        let is_active = self.panes[side].active().id == tab;
        let row_h = self.row_h();
        let Some(t) = self.panes[side]
            .items_mut()
            .iter_mut()
            .find(|t| t.id == tab)
        else {
            return Task::none();
        };
        if t.brief {
            let (rows, cols) = t.brief_grid(row_h);
            let len = t.panel.entries().len();
            t.col = viewport::brief_first_col(len, t.panel.cursor(), rows, cols, t.col);
            return Task::none();
        }
        match viewport::scroll_to_cursor(t.panel.cursor(), row_h, t.offset, t.height) {
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

    fn say(&mut self, kind: StatusKind, text: String) {
        self.status = Some(Status { kind, text });
    }

    /// A background job finished fine: drop its "working…", not a newer message.
    // ponytail: two mounts at once share one Busy slot; the first result clears it.
    fn clear_busy(&mut self) {
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.kind == StatusKind::Busy)
        {
            self.status = None;
        }
    }

    /// The status bar text (tests).
    #[cfg(test)]
    fn msg(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.text.as_str())
    }

    fn set_drawer(&mut self, d: Option<Drawer>) {
        self.core.window.show_context = d.is_some();
        self.drawer = d;
    }

    /// F1 / About / Ctrl+,: open that drawer, or close it if it is the one open.
    fn toggle_drawer(&mut self, action: Action) {
        let d = match action {
            Action::Help => Drawer::Help,
            Action::About => Drawer::About,
            Action::Donate => Drawer::Donate,
            _ => Drawer::Settings(SettingsForm {
                viewer: self.config.viewer.join(" "),
                editor: self.config.editor.join(" "),
                home: self
                    .config
                    .home_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            }),
        };
        let same = self
            .drawer
            .as_ref()
            .is_some_and(|cur| std::mem::discriminant(cur) == std::mem::discriminant(&d));
        self.set_drawer((!same).then_some(d));
    }

    /// A change in the settings drawer: written like a hand edit of the config, applied now.
    fn set(&mut self, s: Setting) -> Task<Message> {
        let mut c = self.config.clone();
        let form = match &mut self.drawer {
            Some(Drawer::Settings(f)) => Some(f),
            _ => None,
        };
        let command = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        match s {
            Setting::Language(i) => {
                c.language = i
                    .checked_sub(1)
                    .and_then(|i| config::LANGUAGES.get(i))
                    .map_or(String::new(), |l| l.to_string());
            }
            Setting::Theme(i) => c.app_theme = config::AppTheme::ALL[i.min(2)],
            Setting::Skin(i) => c.skin = config::Skin::ALL[i.min(1)],
            Setting::ShowFkeys(b) => c.show_fkeys = b,
            Setting::ShowCmdline(b) => c.show_cmdline = b,
            Setting::InternalViewer(b) => c.internal_viewer = b,
            Setting::ShowHidden(b) => c.show_hidden = b,
            Setting::LastTabHome(b) => {
                c.last_tab_close = if b { LastTab::Home } else { LastTab::Nothing };
            }
            Setting::HomeDir(v) => {
                c.home_dir = (!v.trim().is_empty()).then(|| PathBuf::from(v.trim()));
                if let Some(f) = form {
                    f.home = v;
                }
            }
            Setting::Viewer(v) => {
                c.viewer = command(&v);
                if let Some(f) = form {
                    f.viewer = v;
                }
            }
            Setting::Editor(v) => {
                c.editor = command(&v);
                if let Some(f) = form {
                    f.editor = v;
                }
            }
            Setting::PackFormat(i) => {
                if let Some(f) = Format::PACK.get(i) {
                    c.pack_format = f.ext().into();
                }
            }
        }
        if let Some(h) = &self.config_handler
            && let Err(e) = c.write_entry(h)
        {
            log::warn!("config: {e}");
        }
        self.apply_config(c)
    }

    /// New settings, from the settings drawer or the config watcher (a hand edit).
    fn apply_config(&mut self, c: Config) -> Task<Message> {
        let old = std::mem::replace(&mut self.config, c);
        let mut tasks = Vec::new();
        if old.language != self.config.language {
            let system = i18n_embed::DesktopLanguageRequester::requested_languages();
            crate::i18n::init(&config::languages(&self.config.language, system));
            self.core.window.header_title = fl!("app-title");
            self.about = drawer::about();
        }
        if old.app_theme != self.config.app_theme {
            tasks.push(cosmic::command::set_theme(self.config.app_theme.theme()));
        }
        if old.show_hidden != self.config.show_hidden {
            tasks.push(self.apply_hidden());
        }
        if old.skin != self.config.skin {
            // Offsets are in pixels of the old row height: bring each cursor back into view.
            let scale = self.config.skin.row_h() / old.skin.row_h();
            for side in 0..2 {
                // Hidden tabs too: they are shown with their stored offset.
                for t in self.panes[side].items_mut() {
                    t.offset *= scale;
                }
                let tab = self.panes[side].active().id;
                tasks.push(self.reveal(side, tab));
                tasks.push(self.restore_scroll(side));
            }
        }
        Task::batch(tasks)
    }

    /// Mounts change rarely and procfs never blocks: re-read with every listing.
    /// gvfs network mounts are dirs under `$XDG_RUNTIME_DIR/gvfs`, not separate mounts.
    fn refresh_mounts(&mut self) {
        if let Ok(m) = std::fs::read_to_string("/proc/self/mounts") {
            self.drives = drives::parse(&m, &self.home);
        }
        if let Some(gvfs) = mount::gvfs_root() {
            self.drives.extend(mount::gvfs_drives(&gvfs));
        }
    }

    /// Ctrl+Shift+F: unmount the removable or network drive holding the active panel. Not while a
    /// job runs: it may be copying from or to that drive.
    fn disconnect(&mut self, side: usize) -> Task<Message> {
        let cwd = self.panes[side].active().target();
        let gvfs = mount::gvfs_root();
        let Some(i) = drives::containing(&self.drives, &cwd)
            .filter(|&i| mount::removable(&self.drives[i].path, gvfs.as_deref()))
        else {
            return Task::none();
        };
        if self.job.is_some() {
            self.say(StatusKind::Error, fl!("job-running")); // it may be using that disk
            return Task::none();
        }
        let root = self.drives[i].path.clone();
        self.say(StatusKind::Busy, fl!("unmounting"));
        let r = root.clone();
        blocking(
            move || mount::unmount(&r),
            move |res| Message::Unmounted(root.clone(), res),
        )
    }

    /// Enter / click on entry `i` of the open list.
    fn pick(&mut self, i: usize) -> Task<Message> {
        let Some(Dialog::List {
            kind, side, items, ..
        }) = self.dialog.take()
        else {
            return Task::none();
        };
        if kind == ListKind::Commands {
            if let Some(item) = items.get(i) {
                self.cmdline.clone_from(&item.label);
                return self.cmd_focus();
            }
            return Task::none();
        }
        let what = items.get(i).map(|it| it.kind);
        if what == Some(Item::Configure) {
            let list = self.config.hotlist.clone();
            self.dialog = Some(Dialog::Hotlist(Box::new(HotEdit::new(side, list))));
            return Task::none();
        }
        if what == Some(Item::Sep) {
            return Task::none();
        }
        if what == Some(Item::Add) {
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
            Some(item) if item.kind == Item::Mount => {
                self.say(StatusKind::Busy, fl!("mounting"));
                let device = item.path.to_string_lossy().into_owned();
                blocking(
                    move || mount::mount_device(&device),
                    move |r| Message::Mounted(side, r),
                )
            }
            Some(item) => self.go_to(side, item.path.clone()),
            None => Task::none(),
        }
    }

    /// Hotlist rows as TC's menu: the favourites (row = index in the config), a line,
    /// "add current dir", "configure…".
    fn hotlist_items(&self) -> Vec<ListItem> {
        let own = |label, kind| ListItem {
            label,
            path: PathBuf::new(),
            kind,
        };
        let mut items: Vec<ListItem> = self
            .config
            .hotlist
            .iter()
            .map(|e| ListItem {
                label: e.name.clone(),
                path: e.path.clone(),
                kind: if hotlist::is_sep(e) {
                    Item::Sep
                } else {
                    Item::Dir
                },
            })
            .collect();
        if !items.is_empty() {
            items.push(own(String::new(), Item::Sep));
        }
        items.push(own(fl!("hotlist-add"), Item::Add));
        items.push(own(fl!("hotlist-configure"), Item::Configure));
        items
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

    fn save_connections(&mut self, list: Vec<String>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_connections(h, list) {
                    log::warn!("config: {e}");
                }
            }
            None => self.config.connections = list,
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
            if fresh[*cursor].kind == Item::Sep {
                *cursor += 1; // the line before "add": land on "add"
            }
            *items = fresh;
        }
        Task::none()
    }

    /// The command line takes input: shown, and nothing modal or in front of the panels.
    fn cmd_ready(&self) -> bool {
        self.config.show_cmdline
            && self.dialog.is_none()
            && self.drawer.is_none()
            && self.lister.is_none()
            && self.search.is_none()
            && !self.busy()
    }

    /// Num+ / Num− dialog: ↑ older mask, ↓ newer (TC's drop-down history).
    fn mask_history(&mut self, action: Action) -> Option<Task<Message>> {
        let Some(Dialog::Mask { input, .. }) = &mut self.dialog else {
            return None;
        };
        let step = match action {
            Action::Up => cmdline::previous(&self.masks, input),
            Action::Down => cmdline::next(&self.masks, input),
            _ => return None,
        };
        if let Some(m) = step {
            *input = m;
        }
        Some(widget::text_input::move_cursor_to_end(
            self.input_id.clone(),
        ))
    }

    /// A job's dialog is up: the panels wait. A job in the background leaves them free.
    fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.hidden)
    }

    /// Command line keys and the panel keys it changes (Enter, Space with text typed).
    fn cmd_action(&mut self, side: usize, action: Action) -> Option<Task<Message>> {
        if !self.cmd_ready() {
            return matches!(
                action,
                Action::CmdName
                    | Action::CmdPath
                    | Action::CmdCwd
                    | Action::CmdPrevious
                    | Action::CmdHistory
            )
            .then(Task::none);
        }
        let panel = &self.panes[side].active().panel;
        let word = match action {
            Action::Mark if !self.cmdline.is_empty() => {
                self.cmdline.push(' ');
                None
            }
            Action::CmdName | Action::CmdPath => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                Some(cmdline::quote(&if action == Action::CmdPath {
                    panel.cwd().join(&e.os_name).display().to_string()
                } else {
                    e.name.clone()
                }))
            }
            Action::CmdCwd => Some(cmdline::quote(&panel.cwd().display().to_string())),
            Action::CmdPrevious => {
                self.cmdline = cmdline::previous(&self.commands, &self.cmdline)?;
                None
            }
            Action::CmdHistory => {
                if !self.commands.is_empty() {
                    self.dialog = Some(Dialog::List {
                        kind: ListKind::Commands,
                        side,
                        cursor: 0,
                        items: (self.commands.iter())
                            .map(|c| ListItem {
                                label: c.clone(),
                                path: PathBuf::new(),
                                kind: Item::Dir,
                            })
                            .collect(),
                    });
                }
                return Some(Task::none());
            }
            _ => return None,
        };
        if let Some(w) = word {
            self.cmdline = cmdline::append(&self.cmdline, &w);
        }
        Some(self.cmd_focus())
    }

    /// Focus the command line with the cursor at the end (an already focused field keeps its cursor).
    fn cmd_focus(&self) -> Task<Message> {
        Task::batch([
            widget::text_input::focus(self.cmd_id.clone()),
            widget::text_input::move_cursor_to_end(self.cmd_id.clone()),
        ])
    }

    /// Run the command line in the active panel's dir (`cd` changes the dir instead).
    fn cmd_run(&mut self, terminal: bool) -> Task<Message> {
        if !self.cmd_ready() {
            return Task::none();
        }
        let line = std::mem::take(&mut self.cmdline);
        let side = self.active;
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let Some(cmd) = cmdline::parse(&line, &cwd, &self.home) else {
            return unfocus();
        };
        self.commands = cmdline::remember(&self.commands, &line);
        let task = match cmd {
            Cmd::Cd(dir) => self.go_to(side, dir),
            Cmd::Run(line) => {
                let term = terminal.then_some(self.config.terminal.as_slice());
                if let Err(e) = spawn_in(&cmdline::argv(&line, &cwd, term), &cwd) {
                    self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                }
                Task::none()
            }
        };
        Task::batch([unfocus(), task])
    }

    /// Ctrl+↑: the dir under the cursor (an archive as a dir, ".." the parent), else the panel's own.
    fn tab_target(&self, side: usize) -> PathBuf {
        let panel = &self.panes[side].active().panel;
        if let Some((path, _)) = panel.enter_path() {
            return path;
        }
        let cwd = panel.cwd();
        match panel.current() {
            Some(e)
                if archive::split_path(cwd).is_none()
                    && Format::detect(&e.name).is_some_and(Format::is_tree) =>
            {
                cwd.join(&e.os_name)
            }
            _ => cwd.to_path_buf(),
        }
    }

    fn go_to(&mut self, side: usize, path: PathBuf) -> Task<Message> {
        self.active = side;
        self.load(side, path, None)
    }

    /// The newly shown tab was not watched while hidden: restore its scroll and rescan it.
    fn tab_switched(&mut self, side: usize) -> Task<Message> {
        let t = self.panes[side].active();
        let (dir, tab) = (t.target(), t.id);
        // A hidden Brief tab missed the resizes: regroup its columns.
        let reveal = if t.brief {
            self.reveal(side, tab)
        } else {
            Task::none()
        };
        Task::batch([
            reveal,
            self.restore_scroll(side),
            self.reload(side, dir, None),
        ])
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

/// Scroll the viewer and record the offset: iced reports nothing when the content fits, and a
/// stale offset would leave the rows out of view.
fn lister_scroll(l: &mut Lister, x: f32, y: f32) -> Task<Message> {
    l.offset = (x, y);
    scrollable::scroll_to(
        l.scroll.clone(),
        AbsoluteOffset {
            x: Some(x),
            y: Some(y),
        },
    )
}

/// Window events → messages. Panel keys only when no widget took the event (`Ignored`).
fn route_event(
    event: cosmic::iced::Event,
    status: event::Status,
    window: cosmic::iced::window::Id,
) -> Option<Message> {
    match event {
        // Any status: a focused text_input captures Escape to unfocus itself, and the dialog must still close.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            modifiers,
            ..
        }) if modifiers.is_empty() => Some(Message::Escape(window)),
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if status == event::Status::Ignored => keymap::action(&key, physical_key, modifiers)
            .map(Message::Key)
            .or_else(|| {
                let c = typed_char(&key, modifiers);
                match (keymap::lister_key(&key, physical_key, modifiers), c) {
                    (Some(k), Some(c)) => Some(Message::Letter(k, c)),
                    (Some(k), None) => Some(Message::ListerKey(k)),
                    (None, c) => c.map(Message::CmdType),
                }
            }),
        // A focused text field captures every key but Up/Down/Tab; pass on the ones it has no use for.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if not_for_text(&key, physical_key, modifiers) => {
            keymap::action(&key, physical_key, modifiers)
                .map(Message::FieldKey)
                .or_else(|| {
                    // Shift+F3 in the viewer's search field: previous match.
                    keymap::lister_key(&key, physical_key, modifiers)
                        .filter(|k| *k == ListerKey::FindPrev)
                        .map(Message::ListerKey)
                })
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

/// A printable character typed without Ctrl/Alt/Super: TC sends it to the command line.
fn typed_char(key: &keyboard::Key, mods: Modifiers) -> Option<char> {
    let keyboard::Key::Character(s) = key else {
        return None;
    };
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        // Space is a panel key (marking); Shift+Space must not start the line with a blank.
        (Some(c), None)
            if !c.is_control() && c != ' ' && !(mods.control() || mods.alt() || mods.logo()) =>
        {
            Some(c)
        }
        _ => None,
    }
}

/// Take the keyboard focus from every text field.
fn unfocus() -> Task<Message> {
    widget::text_input::focus(widget::Id::unique())
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

/// Run a gio call off the UI thread.
fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, mount::Error> + Send + 'static,
    msg: impl FnOnce(Result<T, mount::Error>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(f)
                .await
                .unwrap_or_else(|e| Err(mount::Error::Failed(e.to_string())))
        },
        move |r| cosmic::Action::App(msg(r)),
    )
}

/// Alt+F1/F2: the volumes gio knows, appended to the open list when they arrive (~50 ms, D-Bus).
fn list_volumes(side: usize) -> Task<Message> {
    blocking(mount::list, move |r| {
        Message::Volumes(side, r.unwrap_or_default())
    })
}

fn mount_error(e: &mount::Error) -> String {
    match e {
        mount::Error::NeedPassword => fl!("mount-need-password"),
        mount::Error::WrongPassword => fl!("mount-wrong-password"),
        mount::Error::Question(q) => fl!("mount-question", text = q.replace('\n', " ")),
        mount::Error::Failed(s) => s.clone(),
        mount::Error::Cancelled => fl!("connect-cancelled"),
    }
}

/// A dir's entries, or the search results fed to the panel (re-read: deleted ones drop out).
fn read_listing(
    path: &Path,
    show_hidden: bool,
    results: Option<&Vec<PathBuf>>,
) -> Result<Vec<Entry>, String> {
    match results {
        Some(r) => Ok(listing::entries(r, path)),
        None => listing::scan(path, show_hidden).map_err(|e| e.to_string()),
    }
}

/// The dialog's search fields as a query (`hidden` is the panel's).
/// A found path through an archive (`/x/a.zip/docs/f`): an ancestor has an archive's name. By
/// name only — a stat per result would stall on a million of them.
/// Count dirs `names` in `cwd` in the background; each sends its own `Message::DirSize`.
// ponytail: never stopped (a huge tree keeps counting after leaving the dir, the result is
// dropped) and one blocking task per dir; add a per-tab stop / a queue if it matters.
fn count_dirs(side: usize, tab: u64, cwd: PathBuf, names: Vec<OsString>) -> Task<Message> {
    Task::batch(names.into_iter().map(|name| {
        let path = cwd.join(&name);
        let cwd = cwd.clone();
        Task::perform(
            async move {
                let stop = AtomicBool::new(false);
                tokio::task::spawn_blocking(move || shagoff_core::props::usage(&[path], &stop))
                    .await
                    .ok()
                    .flatten()
            },
            move |u| {
                let bytes = u.map_or(0, |u| u.bytes);
                cosmic::Action::App(Message::DirSize(
                    side,
                    tab,
                    cwd.clone(),
                    name.clone(),
                    bytes,
                ))
            },
        )
    }))
}

fn inside_archive(p: &Path) -> bool {
    p.ancestors().skip(1).any(|a| {
        a.file_name()
            .and_then(|n| n.to_str())
            .and_then(Format::detect)
            .is_some_and(Format::is_tree)
    })
}

fn find_query(f: &dialogs::Find, text: String) -> Result<shagoff_core::search::Query, String> {
    let number = |s: &str, what: String| -> Result<Option<u64>, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        s.parse::<u64>()
            .map(Some)
            .map_err(|_| fl!("find-bad-number", field = what, value = s))
    };
    let kb = |n: Option<u64>| n.map(|n| n.saturating_mul(1024));
    let days = number(&f.days, fl!("find-days"))?;
    let (min, max) = (
        kb(number(&f.min_size, fl!("find-min-size"))?),
        kb(number(&f.max_size, fl!("find-max-size"))?),
    );
    if let (Some(a), Some(b)) = (min, max)
        && a > b
    {
        return Err(fl!("find-bad-range"));
    }
    let regex = match (f.regex, text.is_empty()) {
        (true, false) => Some(
            shagoff_core::search::regex(&text, f.case_sensitive)
                .map_err(|e| fl!("find-bad-regex", err = e))?,
        ),
        _ => None,
    };
    // `*` (the mask's default) and nothing mean any name, not a broken regex.
    let name_regex = match f.name_regex && !matches!(f.mask.trim(), "" | "*") {
        true => Some(
            shagoff_core::search::name_regex(f.mask.trim(), f.case_sensitive)
                .map_err(|e| fl!("find-bad-regex", err = e))?,
        ),
        false => None,
    };
    Ok(shagoff_core::search::Query {
        mask: Mask::parse_case(&f.mask, f.case_sensitive),
        name_regex,
        archives: f.archives,
        text: (regex.is_none() && !text.is_empty()).then_some(text),
        case_sensitive: f.case_sensitive,
        regex,
        min_size: min,
        max_size: max,
        // Huge day counts reach before 1970: from the start of time then (no overflow panic).
        newer_than: days.map(|d| {
            SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(d.saturating_mul(86400)))
                .unwrap_or(SystemTime::UNIX_EPOCH)
        }),
        ..Default::default()
    })
}

/// A typed target that lies inside an archive (`a.zip/x`). The archive file itself is not: packing
/// or copying onto an existing `a.zip` asks to replace it, as for any file.
fn into_archive(p: &Path) -> bool {
    archive::split_path(p).is_some_and(|(_, inner)| !inner.as_os_str().is_empty())
}

/// Default F5/F6 target: the other pane's dir with a trailing `/` (so it reads as "into this dir").
fn dir_input(dir: &Path) -> String {
    let s = dir.display().to_string();
    if s.ends_with('/') { s } else { s + "/" }
}

/// Run `argv` without blocking the UI or leaving a zombie.
fn spawn_detached(argv: &[OsString]) -> std::io::Result<()> {
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let child = std::process::Command::new(prog).args(args).spawn()?;
    reap(child);
    Ok(())
}

/// A command line in `dir`, its output dropped (TC closes the console too).
fn spawn_in(argv: &[String], dir: &Path) -> std::io::Result<()> {
    use std::process::Stdio;
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let child = std::process::Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    reap(child);
    Ok(())
}

fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || child.wait());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LastTab, State};
    use crate::dialogs::ListKind;
    use cosmic::iced::keyboard::key::{Code, Named, Physical};
    use cosmic::iced::keyboard::{Key, Location};
    use shagoff_core::archive::Format;
    use shagoff_core::diff::Outcome;
    use shagoff_core::drives::Drive;
    use shagoff_core::lister::Mode;
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
            owner: None,
            target: None,
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
                    ..PaneState::default()
                },
                PaneState {
                    tabs: vec![tmp.path().join("gone")],
                    active: 0,
                    ..PaneState::default()
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
                    ..PaneState::default()
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
        let err = app.msg().unwrap_or_default().to_string();
        assert_eq!(err, fl!("broken-link", name = "dangling"));
    }

    /// Pane 0 in a temp dir with `sub/`, `a.txt` ("alpha\nfoo\n"), `b.bin`; cursor on `a.txt`.
    fn lister_app(config: Config) -> (App, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha\nfoo\n").unwrap();
        std::fs::write(tmp.path().join("b.bin"), b"x\0y").unwrap();
        let mut app = app_with(config, State::default());
        let _ = app.load(0, tmp.path().into(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let entries = listing::scan(tmp.path(), false).unwrap();
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: tmp.path().into(),
            focus: Some("a.txt".into()),
            result: Ok(entries),
            space: None,
        });
        (app, tmp)
    }

    /// What the background read would deliver for the viewer's current file.
    fn lister_loaded(app: &mut App, dir: &Path) {
        let l = app.lister.as_ref().unwrap();
        let (id, name) = (l.id, l.name.clone());
        let loaded = lister::Loaded::read(&dir.join(&name), &name);
        let _ = app.update(Message::ListerLoaded(id, Arc::new(loaded)));
    }

    #[test]
    fn f3_opens_the_viewer_and_esc_closes_it() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        assert_eq!(app.lister.as_ref().map(|l| l.name.as_str()), Some("a.txt"));
        lister_loaded(&mut app, tmp.path());
        assert_eq!(app.lister.as_ref().unwrap().rows(), 2);
        // Panel keys are the viewer's now: F8 does not ask to delete.
        let _ = app.update(Message::Key(Action::Delete));
        assert!(app.dialog.is_none());
        let _ = app.update(Message::DialogCancel);
        assert!(app.lister.is_none());
    }

    #[test]
    fn f3_without_the_builtin_viewer_runs_the_program() {
        let config = Config {
            internal_viewer: false,
            viewer: vec!["/nonexistent/viewer".into()], // never launch anything from a test
            ..Config::default()
        };
        let (mut app, _tmp) = lister_app(config);
        let _ = app.update(Message::Key(Action::View));
        assert!(app.lister.is_none());
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
    }

    #[test]
    fn viewer_n_and_p_walk_files_skipping_dirs() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        let _ = app.update(Message::ListerKey(ListerKey::Next));
        assert_eq!(app.lister.as_ref().unwrap().name, "b.bin");
        let cur = |app: &App| app.panes[0].active().panel.current().unwrap().name.clone();
        assert_eq!(cur(&app), "b.bin");
        lister_loaded(&mut app, tmp.path());
        assert_eq!(app.lister.as_ref().unwrap().mode, Mode::Hex); // NUL inside
        let _ = app.update(Message::ListerKey(ListerKey::Next)); // last file: stays
        assert_eq!(cur(&app), "b.bin");
        let _ = app.update(Message::ListerKey(ListerKey::Prev));
        let _ = app.update(Message::ListerKey(ListerKey::Prev)); // `sub` and `..` skipped
        assert_eq!(cur(&app), "a.txt");
        assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
    }

    #[test]
    fn regression_mode_switch_resets_offset() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        lister_loaded(&mut app, tmp.path());
        // Scrolled far down in another mode; a short text fits, so iced reports no scroll.
        let _ = app.update(Message::ListerScrolled(30.0, 4700.0, 400.0));
        let _ = app.update(Message::ListerMode(Mode::Hex));
        assert_eq!(app.lister.as_ref().unwrap().offset, (0.0, 0.0));
        let _ = app.update(Message::Key(Action::End));
        assert_eq!(app.lister.as_ref().unwrap().offset, (0.0, 0.0)); // 1 hex row fits
    }

    #[test]
    fn settings_field_keys_do_not_reach_the_viewer() {
        let (mut app, _tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        let _ = app.update(Message::Key(Action::Settings));
        assert!(app.drawer.is_some());
        let _ = app.update(Message::FieldKey(Action::Mkdir)); // F7 typed in a settings field
        assert!(!app.lister.as_ref().unwrap().searching);
    }

    #[test]
    fn viewer_next_onto_a_broken_link_keeps_cursor_and_file() {
        let (mut app, tmp) = lister_app(Config::default());
        std::os::unix::fs::symlink("nowhere", tmp.path().join("c_dangling")).unwrap();
        let _ = app.load(0, tmp.path().into(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: tmp.path().into(),
            focus: Some("b.bin".into()),
            result: Ok(listing::scan(tmp.path(), false).unwrap()),
            space: None,
        });
        let _ = app.update(Message::Key(Action::View));
        let _ = app.update(Message::ListerKey(ListerKey::Next));
        assert_eq!(app.lister.as_ref().unwrap().name, "b.bin");
        let cur = app.panes[0].active().panel.current().unwrap().name.clone();
        assert_eq!(cur, "b.bin");
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
    }

    #[test]
    fn regression_viewer_arrows_scroll_sideways() {
        // ← / → are Brief keys in the main table now; the viewer still takes them.
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        lister_loaded(&mut app, tmp.path());
        let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Hex)));
        let _ = app.update(Message::Key(Action::Right));
        assert!(app.lister.as_ref().unwrap().offset.0 > 0.0);
        let _ = app.update(Message::Key(Action::Left));
        assert_eq!(app.lister.as_ref().unwrap().offset.0, 0.0);
        assert_eq!(app.panes[0].active().panel.cursor(), 2); // still on a.txt
    }

    #[test]
    fn viewer_modes_and_search() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        lister_loaded(&mut app, tmp.path());
        let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Hex)));
        assert_eq!(app.lister.as_ref().unwrap().mode, Mode::Hex);
        let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Text)));
        // F3 with nothing to find opens the field; Esc closes the field, not the viewer.
        let _ = app.update(Message::Key(Action::View));
        assert!(app.lister.as_ref().unwrap().searching);
        let _ = app.update(Message::DialogCancel);
        assert!(app.lister.as_ref().is_some_and(|l| !l.searching));
        let _ = app.update(Message::Key(Action::Mkdir)); // F7
        let _ = app.update(Message::ListerQuery("FOO".into()));
        let _ = app.update(Message::ListerFind);
        let l = app.lister.as_ref().unwrap();
        assert_eq!((l.hit, l.searching), (Some(1), false));
        let _ = app.update(Message::Key(Action::View)); // F3: no further match
        assert_eq!(
            app.msg(),
            Some(fl!("lister-not-found", query = "FOO").as_str())
        );
    }

    /// Row height of the default skin, for tests that size the list in rows.
    const ROW: f32 = 20.0;

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
        let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, height)));
        app
    }

    #[test]
    fn default_skin_row_matches_the_tests() {
        assert_eq!(config::Skin::default().row_h(), ROW);
    }

    #[test]
    fn switching_skin_keeps_the_cursor_on_screen() {
        let mut app = tall_list(4.5 * ROW);
        let _ = app.update(Message::Key(Action::End)); // row 20
        let _ = app.update(Message::Key(Action::Settings));
        let _ = app.update(Message::Setting(crate::drawer::Setting::Skin(1)));
        assert_eq!(app.config.skin, config::Skin::Modern);
        let (t, row_h) = (app.panes[0].active(), app.row_h());
        let bottom = 21.0 * row_h;
        assert!(
            t.offset <= 20.0 * row_h && t.offset + t.height >= bottom,
            "offset {}",
            t.offset
        );
    }

    #[test]
    fn brief_view_moves_by_columns() {
        // ".." + 20 files; 4.5 rows high = 4 rows, 500 px = 2 columns on screen
        let mut app = tall_list(4.5 * ROW);
        let cur = |app: &App| app.panes[0].active().panel.cursor();
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), 0); // Full view: ← / → do nothing
        let _ = app.update(Message::Key(Action::ViewBrief));
        assert!(app.panes[0].active().brief);
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), 4);
        let _ = app.update(Message::Key(Action::PageDown));
        assert_eq!(cur(&app), 12);
        let _ = app.update(Message::Key(Action::Left));
        assert_eq!(cur(&app), 8);
        let _ = app.update(Message::Key(Action::End));
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), 20);
        assert_eq!(app.panes[0].active().col, 4); // 6 columns, the last two on screen
        let _ = app.update(Message::BriefWheel(
            0,
            mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        ));
        assert_eq!(app.panes[0].active().col, 3);
        let _ = app.update(Message::Key(Action::Home));
        assert_eq!(app.panes[0].active().col, 0);
        let _ = app.update(Message::Key(Action::ViewFull));
        assert!(!app.panes[0].active().brief);
    }

    #[test]
    fn brief_columns_follow_a_resize_and_touchpad_steps_add_up() {
        let mut app = tall_list(4.5 * ROW); // 4 rows
        let _ = app.update(Message::Key(Action::ViewBrief));
        let _ = app.update(Message::Key(Action::End)); // entry 20, column 5
        assert_eq!(app.panes[0].active().col, 4);
        // taller: 10 rows → 3 columns, both fit; the old first column would hide the cursor
        let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 10.5 * ROW)));
        assert_eq!(app.panes[0].active().col, 1);
        let px = |y| Message::BriefWheel(0, mouse::ScrollDelta::Pixels { x: 0.0, y });
        let _ = app.update(px(5.0));
        let _ = app.update(px(5.0));
        assert_eq!(app.panes[0].active().col, 1); // 10 px: less than a row
        let _ = app.update(px(15.0));
        assert_eq!(app.panes[0].active().col, 0);
    }

    #[test]
    fn regression_click_on_half_visible_row_scrolls_it_in() {
        let mut app = tall_list(4.5 * ROW); // rows 0..4 full, row 4 cut in half
        let _ = app.update(Message::Click(0, 4));
        let t = app.panes[0].active();
        assert!(
            t.offset + t.height >= 5.0 * app.row_h(),
            "offset {}",
            t.offset
        );
    }

    #[test]
    fn regression_shrinking_window_keeps_cursor_visible() {
        let mut app = tall_list(400.0);
        let _ = app.update(Message::Click(0, 10));
        let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 100.0))); // window got smaller
        let t = app.panes[0].active();
        assert!(
            t.offset + t.height >= 11.0 * app.row_h(),
            "offset {}",
            t.offset
        );
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
        let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 150.0)));
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
    fn shift_f5_offers_the_name_in_the_same_dir() {
        let mut app = files_app(&["a.txt"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CopySame));
        let Some(Dialog::Input {
            op, input, sources, ..
        }) = &app.dialog
        else {
            panic!("no dialog");
        };
        assert_eq!((*op, input.as_str()), (InputOp::Copy, "a.txt"));
        assert_eq!(sources, &[std::env::temp_dir().join("a.txt")]);
        // on ".." there is nothing to copy
        let mut app = files_app(&["a.txt"]);
        let _ = app.update(Message::Key(Action::CopySame));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn conflict_rename_sends_the_typed_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        let mut app = files_app(&[]);
        let info = |p: PathBuf| ops::FileInfo {
            path: p,
            size: 0,
            mtime: std::time::SystemTime::UNIX_EPOCH,
        };
        let (reply, rx) = std::sync::mpsc::channel();
        let _ = app.on_job_event(jobs::Event::Conflict {
            src: info("/x/a.txt".into()),
            dst: info(tmp.path().join("a.txt")),
            reply,
        });
        let Some(Dialog::Conflict { name, .. }) = &app.dialog else {
            panic!("no dialog");
        };
        assert_eq!(name, "a (1).txt");
        let _ = app.update(Message::DialogInput("b.txt".into()));
        let _ = app.update(Message::Resolve(Resolution::Rename("b.txt".into())));
        assert_eq!(rx.recv().unwrap(), Resolution::Rename("b.txt".into()));
        assert!(app.dialog.is_none());
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
    fn a_job_in_the_background_frees_the_panels_but_not_for_a_second_job() {
        let mut app = files_app(&["a", "b", "c"]);
        let dir = app.panes[0].active().panel.cwd().to_path_buf();
        let gone = vec![dir.join("a")];
        let _ = app.start_job(
            0,
            OpKind::Delete,
            Job::Delete {
                paths: gone,
                permanent: true,
            },
            None,
        );
        assert!(app.busy() && app.dialog().is_some());
        let before = cursor_name(&app);
        let _ = app.update(Message::Key(Action::Down)); // the dialog is up: the panel waits
        assert_eq!(cursor_name(&app), before);
        let _ = app.update(Message::JobHide);
        assert!(!app.busy() && app.dialog().is_none());
        let _ = app.update(Message::Key(Action::Down));
        assert_ne!(cursor_name(&app), before);
        let _ = app.update(Message::DialogCancel); // Esc: not for the hidden job
        assert!(!app.job.as_ref().unwrap().cancel.load(Ordering::Relaxed));
        let again = vec![dir.join("b")];
        let _ = app.start_job(
            0,
            OpKind::Delete,
            Job::Delete {
                paths: again,
                permanent: true,
            },
            None,
        );
        assert_eq!(app.msg(), Some(fl!("job-running").as_str()));
        let _ = app.update(Message::JobShow);
        assert!(app.busy());
    }

    #[test]
    fn mask_dialog_remembers_masks() {
        let mut app = app_with(Config::default(), State::default());
        let mask = |app: &App| match &app.dialog {
            Some(Dialog::Mask { input, .. }) => input.clone(),
            _ => panic!("no mask dialog"),
        };
        for m in ["*.rs", "*.txt"] {
            let _ = app.update(Message::Key(Action::SelectGroup));
            let _ = app.update(Message::DialogInput(m.into()));
            let _ = app.update(Message::DialogSubmit);
        }
        assert_eq!(app.masks, ["*.txt", "*.rs"]);
        let _ = app.update(Message::Key(Action::UnselectGroup));
        assert_eq!(mask(&app), "*.txt"); // opens with the last one
        let _ = app.update(Message::FieldKey(Action::Up));
        assert_eq!(mask(&app), "*.rs");
        let _ = app.update(Message::FieldKey(Action::Down));
        assert_eq!(mask(&app), "*.txt");
        // regression: the field lets ↑ / ↓ through as plain keys, which went nowhere
        let _ = app.update(Message::Key(Action::Up));
        assert_eq!(mask(&app), "*.rs");
    }

    #[test]
    fn hotlist_add_once_delete_clamps() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // empty: row 0 is "add current dir"
        let paths: Vec<_> = app.config.hotlist.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, std::slice::from_ref(&cwd));
        assert!(app.dialog.is_none());

        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Enter)); // add again: no duplicate
        assert_eq!(app.config.hotlist.len(), 1);

        let _ = app.update(Message::Key(Action::Hotlist));
        assert_eq!(hotlist_items(&app).len(), 4); // entry, line, add, configure
        let _ = app.update(Message::Key(Action::Down)); // over the line onto "add"
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 2, .. })));
        let _ = app.update(Message::Key(Action::Delete)); // on the add row: nothing
        assert_eq!(app.config.hotlist.len(), 1);
        let _ = app.update(Message::Key(Action::Up));
        let _ = app.update(Message::Key(Action::Delete)); // remove the entry
        assert!(app.config.hotlist.is_empty());
        assert_eq!(hotlist_items(&app).len(), 2); // still open
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
    }

    #[test]
    fn hotlist_configure_edits_a_copy_saved_on_ok() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Down)); // "configure…"
        let _ = app.update(Message::Key(Action::Enter));
        let _ = app.update(Message::Hot(HotMsg::Add));
        let _ = app.update(Message::Hot(HotMsg::Name("Home".into())));
        assert!(app.config.hotlist.is_empty()); // not until OK
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(
            app.config.hotlist,
            [HotEntry {
                name: "Home".into(),
                path: cwd
            }]
        );
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
        let _ = app.update(Message::Key(Action::Hotlist)); // cursor on the first entry
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

    #[test]
    fn viewer_keys_reach_the_app_but_not_from_a_text_field() {
        let free = route_event(
            cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character("n".into()),
                modified_key: Key::Character("n".into()),
                physical_key: Physical::Code(Code::KeyN),
                location: Location::Standard,
                modifiers: Modifiers::empty(),
                text: None,
                repeat: false,
            }),
            event::Status::Ignored,
            cosmic::iced::window::Id::unique(),
        );
        assert!(
            matches!(free, Some(Message::Letter(ListerKey::Next, 'n'))),
            "{free:?}"
        );
        // Typed into the search field: stays text.
        let typed = press_with(Key::Character("n".into()), Code::KeyN, Modifiers::empty());
        assert!(typed.is_none(), "{typed:?}");
        let prev = press_with(Key::Named(Named::F3), Code::F3, Modifiers::SHIFT);
        assert!(
            matches!(prev, Some(Message::ListerKey(ListerKey::FindPrev))),
            "{prev:?}"
        );
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
        assert!(matches!(msg, Some(Message::Escape(_))), "{msg:?}");
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
        assert!(app.msg().is_none());
    }

    #[test]
    fn paste_into_source_dir_reports_same_file() {
        let (_tmp, mut app, a) = paste_setup();
        listed_at(&mut app, 0, a.parent().unwrap());
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a.clone()]))));
        assert!(app.job.is_none());
        assert_eq!(
            app.msg(),
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
    fn f8_inside_archive_asks_permanent_then_repacks() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Delete)); // F8, not Shift+F8: still no trash
        assert!(matches!(
            app.dialog,
            Some(Dialog::ConfirmDelete {
                permanent: true,
                ..
            })
        ));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    }

    #[test]
    fn f7_and_rename_inside_archive_repack() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Mkdir));
        let _ = app.update(Message::DialogInput("new".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
        app.job = None;
        let _ = app.update(Message::Key(Action::End)); // top.txt
        let _ = app.update(Message::Key(Action::Rename));
        let _ = app.update(Message::DialogInput("renamed.txt".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    }

    #[test]
    fn f6_out_of_archive_moves() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::End));
        let _ = app.update(Message::Key(Action::Move));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    }

    #[test]
    fn regression_f6_of_the_archive_into_itself_is_refused() {
        let (_tmp, mut app, a) = zip_setup(); // cursor on a.zip
        listed_at(&mut app, 1, &a);
        let _ = app.update(Message::Key(Action::Move));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.job.is_none());
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
        assert!(a.exists());
    }

    #[test]
    fn rename_inside_archive_refuses_a_slash() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::End));
        let _ = app.update(Message::Key(Action::Rename));
        let _ = app.update(Message::DialogInput("d/x.txt".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.job.is_none());
        assert_eq!(app.msg(), Some(fl!("rename-bad-name").as_str()));
    }

    #[test]
    fn ctrl_m_inside_archive_stays_read_only() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::End));
        let _ = app.update(Message::Key(Action::MultiRename));
        assert!(app.dialog.is_none());
        assert_eq!(app.msg(), Some(fl!("archive-read-only").as_str()));
    }

    #[test]
    fn edited_file_from_archive_asks_once_per_change() {
        let (tmp, mut app, a) = zip_setup();
        let file = tmp.path().join("out/top.txt");
        std::fs::write(&file, "x").unwrap();
        app.edited.push(Edited {
            file: file.clone(),
            archive: a,
            entry: "top.txt".into(),
            mtime: std::time::UNIX_EPOCH, // "changed since extraction"
        });
        let _ = app.update(Message::EditTick);
        assert!(matches!(app.dialog, Some(Dialog::UpdateArchive { .. })));
        let _ = app.update(Message::DialogCancel);
        let _ = app.update(Message::EditTick);
        assert!(app.dialog.is_none()); // the same change is not asked again
        app.edited[0].mtime = std::time::UNIX_EPOCH;
        let _ = app.update(Message::EditTick);
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
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
    fn f5_into_archive_panel_repacks() {
        let (tmp, mut app, a) = zip_setup();
        std::fs::write(tmp.path().join("out/x"), "1").unwrap();
        listed_at(&mut app, 0, &tmp.path().join("out"));
        listed_at(&mut app, 1, &a);
        let _ = app.update(Message::Key(Action::Down)); // x
        let _ = app.update(Message::Key(Action::Copy));
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
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
    fn space_on_dir_marks_it_and_its_counted_size_goes_into_totals() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // sub
        let _ = app.update(Message::Key(Action::Mark));
        let id = app.panes[0].active().id;
        let msg = Message::DirSize(0, id, tmp.path().into(), "sub".into(), 4096);
        let _ = app.update(msg);
        let p = &app.panes[0].active().panel;
        assert_eq!(p.marked_totals().bytes, 4096);
        assert_eq!(p.dir_size(&p.entries()[1]), Some(4096));
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
    fn find_filters_are_remembered_and_name_regex_checked() {
        let (_tmp, mut app) = results_app();
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindNameRegex);
        let _ = app.update(Message::FindArchives);
        let _ = app.update(Message::FindInput(FindField::Mask, "(".into()));
        let _ = app.update(Message::FindInput(FindField::MinSize, "5".into()));
        let _ = app.update(Message::FindStart);
        let Some(Dialog::Find(f)) = &app.dialog else {
            panic!("no dialog");
        };
        assert!(f.error.is_some() && f.stop.is_none()); // a bad name regex does not start
        // `*` with the regex box on: any name, not an error
        let _ = app.update(Message::FindInput(FindField::Mask, "*".into()));
        let _ = app.update(Message::FindStart);
        assert!(find_dialog(&mut app).error.is_none());
        let _ = app.update(Message::FindInput(FindField::Mask, r"\.rs$".into()));
        let _ = app.update(Message::FindStart);
        app.dialog = None;
        app.save_state();
        let prefs = app.saved.find.clone();
        assert!(prefs.name_regex && prefs.archives);
        assert_eq!(
            (prefs.mask.as_str(), prefs.min_size.as_str()),
            (r"\.rs$", "5")
        );
        let mut app = app_with(Config::default(), app.saved.clone());
        let _ = app.update(Message::Key(Action::FindFiles));
        let Some(Dialog::Find(f)) = &app.dialog else {
            panic!("no dialog");
        };
        assert!(f.name_regex && f.archives && f.min_size == "5");
    }

    #[test]
    fn alt_enter_shows_bits_and_facts_of_the_cursor_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.rs");
        std::fs::write(&f, "x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Properties));
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no dialog");
        };
        assert_eq!((p.mode, p.mixed, p.has_dir), (0o640, 0, false));
        assert!(p.facts.iter().any(|(_, v)| v == "a.rs"));
        assert!(p.facts.iter().any(|(_, v)| v == "text/x-rust"));
        assert!(p.id > 0); // counting started
        let id = p.id;
        let _ = app.update(Message::PropsBit(0o100));
        let u = shagoff_core::props::Usage {
            bytes: 1,
            files: 1,
            dirs: 0,
        };
        let _ = app.update(Message::PropsUsage(id + 1, Some(u))); // stale
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no dialog");
        };
        assert_eq!((p.mode, p.usage), (0o740, None));
        let _ = app.update(Message::Key(Action::Enter)); // applies
        assert!(app.dialog.is_none());
        // on ".." there is nothing to show
        let _ = app.update(Message::Key(Action::Home));
        let _ = app.update(Message::Key(Action::Properties));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn properties_of_files_with_different_modes() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        for (n, m) in [("a.sh", 0o755), ("b.sh", 0o644)] {
            let f = tmp.path().join(n);
            std::fs::write(&f, "").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(m)).unwrap();
        }
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::SelectAll));
        let _ = app.update(Message::Key(Action::Properties));
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no dialog");
        };
        // set on both / differing
        assert_eq!((p.mode, p.mixed), (0o644, 0o111));
        let _ = app.update(Message::PropsBit(0o100)); // owner x: now on for both
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no dialog");
        };
        assert_eq!((p.mode & 0o100, p.touched), (0o100, 0o100));
    }

    #[test]
    fn recursive_properties_start_every_bit_as_leave_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Properties));
        let _ = app.update(Message::PropsRecursive);
        let props = |app: &App| match &app.dialog {
            Some(Dialog::Props(p)) => (p.mode, p.mixed, p.touched),
            _ => panic!("no dialog"),
        };
        assert_eq!(props(&app).1, 0o777);
        // group w: `?` → on even if the dir has it already (the contents may not)
        let _ = app.update(Message::PropsBit(0o020));
        let (m, _, touched) = props(&app);
        assert_eq!((m & 0o020, touched), (0o020, 0o020));
        let _ = app.update(Message::PropsBit(0o020)); // then a plain box: off
        assert_eq!(props(&app).0 & 0o020, 0);
        let _ = app.update(Message::PropsRecursive); // off: the dir's own state again
        assert_eq!(props(&app).1, 0);
    }

    #[test]
    fn right_click_moves_the_cursor_and_keeps_marks_only_on_a_marked_row() {
        let mut app = files_app(&["a", "b", "c"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::MarkDown)); // "a" marked, cursor on "b"
        let _ = app.update(Message::RightClick(0, 1)); // on the marked "a"
        let _ = app.update(Message::RightEmpty(0)); // the list's own handler, same press
        assert!(app.ctx_entry);
        assert_eq!(app.panes[0].active().panel.marked_totals().files, 1);
        let _ = app.update(Message::RightClick(0, 3)); // unmarked "c": the menu is about it
        let _ = app.update(Message::RightEmpty(0));
        assert_eq!(cursor_name(&app), "c");
        assert_eq!(app.panes[0].active().panel.marked_totals().files, 0);
        // a press below the rows: the dir's menu
        let _ = app.update(Message::RightEmpty(1));
        assert!(!app.ctx_entry);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn regression_menu_open_opens_even_with_a_typed_command() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        app.cmdline = "ls -l".into();
        let _ = app.update(Message::RightClick(0, 1));
        let _ = app.update(Message::OpenEntry);
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
        assert_eq!(app.cmdline, "ls -l"); // not run
    }

    #[test]
    fn regression_closing_a_popup_does_not_quit() {
        let app = files_app(&[]);
        let popup = cosmic::iced::window::Id::unique();
        assert!(app.on_close_requested(popup).is_none());
        assert!(matches!(
            app.on_close_requested(app.window_id()),
            Some(Message::Exit)
        ));
    }

    #[test]
    fn found_inside_archives_by_name() {
        assert!(inside_archive(Path::new("/x/a.zip/docs/f.txt")));
        assert!(inside_archive(Path::new("/x/b.tar.gz/f")));
        assert!(!inside_archive(Path::new("/x/a.zip"))); // the archive itself is a file
        assert!(!inside_archive(Path::new("/x/notes.gz/f"))); // .gz is no folder
        assert!(!inside_archive(Path::new("/x/docs/f.txt")));
    }

    #[test]
    fn find_results_to_panel_and_back() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let (a, b) = (tmp.path().join("sub/a.rs"), tmp.path().join("b.rs"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindStart);
        let id = find_dialog(&mut app).id;
        // More than the dialog shows: the panel gets them all.
        let many: Vec<PathBuf> = (0..dialogs::FIND_SHOWN + 5).map(|_| a.clone()).collect();
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, many)));
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(
            id,
            vec![b.clone()],
        )));
        let _ = app.update(Message::FindFeed);
        assert!(app.dialog.is_none());
        let t = app.panes[0].active();
        let (root, paths) = t.results.clone().unwrap();
        assert_eq!(root, tmp.path());
        assert_eq!(paths.len(), dialogs::FIND_SHOWN + 6);
        assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root.clone()));
        let e = read_listing(&root, false, Some(&paths)).unwrap();
        assert!(e.iter().any(|e| e.name == "sub/a.rs"));
        // Backspace leaves the results for the dir that was searched.
        let _ = app.update(Message::Key(Action::Parent));
        let t = app.panes[0].active();
        assert!(t.results.is_none());
        assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root));
    }

    /// Pane 0 shows `tmp` as search results holding `sub/a.rs`.
    fn results_app() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let a = tmp.path().join("sub/a.rs");
        std::fs::write(&a, "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let root = tmp.path().to_path_buf();
        app.panes[0].active_mut().results = Some((root.clone(), Arc::new(vec![a.clone()])));
        let _ = app.reload(0, root.clone(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: root.clone(),
            focus: None,
            result: Ok(listing::entries(&[a], &root)),
            space: None,
        });
        (tmp, app)
    }

    #[test]
    fn regression_feed_uses_the_dir_searched_not_the_edited_field() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindStart);
        let id = find_dialog(&mut app).id;
        let f = tmp.path().join("f");
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, vec![f])));
        let _ = app.update(Message::FindInput(FindField::Dir, "/typo".into()));
        let _ = app.update(Message::FindFeed);
        let root = app.panes[0].active().results.clone().unwrap().0;
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn regression_huge_days_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindInput(
            FindField::Days,
            "99999999999999999".into(),
        ));
        let _ = app.update(Message::FindStart);
        assert!(find_dialog(&mut app).stop.is_some());
        // Bounds the wrong way round: said, not silently nothing.
        let _ = app.update(Message::FindInput(FindField::MinSize, "10".into()));
        let _ = app.update(Message::FindInput(FindField::MaxSize, "5".into()));
        let _ = app.update(Message::FindStart);
        assert!(find_dialog(&mut app).error.is_some());
    }

    #[test]
    fn results_refuse_multi_rename_and_compare_lists() {
        let (_tmp, mut app) = results_app();
        for a in [Action::MultiRename, Action::CompareLists] {
            let _ = app.update(Message::Key(a));
            assert!(app.dialog.is_none(), "{a:?}");
            assert_eq!(
                app.msg(),
                Some(fl!("results-unsupported").as_str()),
                "{a:?}"
            );
        }
    }

    #[test]
    fn regression_going_to_the_searched_dir_leaves_results() {
        let (tmp, mut app) = results_app();
        let _ = app.update(Message::Drive(0, tmp.path().to_path_buf()));
        assert!(app.panes[0].active().results.is_none());
    }

    #[test]
    fn find_refuses_bad_filters_and_regex() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindInput(FindField::MinSize, "abc".into()));
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.stop.is_none() && f.error.is_some());
        let _ = app.update(Message::FindInput(FindField::MinSize, "10".into()));
        let _ = app.update(Message::FindInput(FindField::Text, "(".into()));
        let _ = app.update(Message::FindRegex);
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.stop.is_none() && f.error.is_some());
        let _ = app.update(Message::FindInput(FindField::Text, "a+".into()));
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.stop.is_some() && f.error.is_none());
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
    fn sync_show_buttons_toggle_a_kind() {
        use shagoff_core::sync::Kind;
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::SyncDirs));
        let hidden = |app: &App, k| match &app.dialog {
            Some(Dialog::Sync(s)) => s.hide.contains(&k),
            _ => panic!("no sync dialog"),
        };
        assert!(hidden(&app, Kind::Same) && !hidden(&app, Kind::ToRight)); // = off at first
        let _ = app.update(Message::SyncShow(Kind::ToRight));
        let _ = app.update(Message::SyncShow(Kind::Same));
        assert!(hidden(&app, Kind::ToRight) && !hidden(&app, Kind::Same));
    }

    #[test]
    fn mirror_asks_before_deleting() {
        use shagoff_core::sync::Dir;
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::SyncDirs));
        let _ = app.update(Message::SyncOpt(SyncOpt::Mirror));
        compared(&mut app);
        let s = sync_dlg(&mut app);
        let only_r = s
            .rows
            .iter()
            .find(|r| r.rel == Path::new("only_r"))
            .unwrap();
        assert_eq!(only_r.dir, Dir::Delete);
        // First press only asks (never run here: deleting goes to the user's real trash).
        let _ = app.update(Message::SyncRun);
        assert!(app.job.is_none());
        assert!(sync_dlg(&mut app).confirm);
        // A new compare drops the confirmation.
        compared(&mut app);
        assert!(!sync_dlg(&mut app).confirm);
    }

    #[test]
    fn regression_editing_the_mask_drops_rows_and_confirmation() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::SyncDirs));
        let _ = app.update(Message::SyncOpt(SyncOpt::Mirror));
        compared(&mut app);
        let _ = app.update(Message::SyncRun); // armed
        let _ = app.update(Message::SyncMask("*.rs".into()));
        let s = sync_dlg(&mut app);
        assert!(!s.confirm && s.rows.is_empty());
    }

    #[test]
    fn sync_mask_limits_rows() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::SyncDirs));
        let _ = app.update(Message::SyncMask("only_*".into()));
        compared(&mut app);
        let names: Vec<String> = sync_dlg(&mut app)
            .rows
            .iter()
            .map(|r| r.rel.display().to_string())
            .collect();
        assert_eq!(names, ["only_l", "only_r"]);
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
        assert!(app.msg().is_some());
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
        assert!(app.msg().is_some()); // "identical" note in the status line
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
        assert!(app.msg().is_some());
    }

    #[test]
    fn diff_next_prev_clamp_and_stale_ignored() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("a\nb\nc\nd\n", "A\nb\nC\nd\n", 100);
        let out = Arc::new(Ok(shagoff_core::diff::Outcome::Text(t)));
        let _ = app.update(Message::DiffReady(id + 1, out.clone()));
        assert!(diff_dlg(&mut app).result.is_none());
        let _ = app.update(Message::DiffReady(id, out));
        let _ = app.update(Message::Key(Action::Up));
        assert_eq!(diff_dlg(&mut app).block, 0);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(diff_dlg(&mut app).block, 1); // two blocks: clamped
    }

    #[test]
    fn diff_copy_block_then_save_and_esc_asks_first() {
        let (tmp, mut app) = sync_setup();
        let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
        std::fs::write(&l, "x\nNEW\ny\n").unwrap();
        std::fs::write(&r, "x\nold\ny\n").unwrap();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        let _ = app.update(Message::DiffCopy(true)); // block 0, left → right
        let _ = app.update(Message::DiffSave); // still copying: nothing to save yet
        assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nold\ny\n");
        // What the background copy delivers.
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
        let _ = app.update(Message::DiffOpt(dialogs::DiffOpt::Case)); // ignored while copying
        assert!(!diff_dlg(&mut app).opts.ignore_case);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        assert_eq!(diff_dlg(&mut app).dirty, (false, true)); // changed, unsaved
        let _ = app.update(Message::DialogCancel);
        assert!(app.dialog.is_some()); // first Esc only warns
        let _ = app.update(Message::DiffSave);
        assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nNEW\ny\n");
        assert_eq!(std::fs::read_to_string(&l).unwrap(), "x\nNEW\ny\n");
        assert_eq!(diff_dlg(&mut app).dirty, (false, false));
        let _ = app.update(Message::DialogCancel);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn regression_diff_with_three_marked_does_not_fall_back_to_cursors() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::MarkDown)); // only one marked
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        app.active = 0;
        let _ = app.update(Message::Key(Action::CompareFiles));
        assert!(app.dialog.is_none());
        assert!(app.msg().is_some());
    }

    #[test]
    fn regression_diff_inside_archive_explains() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        assert!(app.dialog.is_none());
        assert_eq!(app.msg(), Some(fl!("diff-in-archive").as_str()));
    }

    #[test]
    fn menu_item_does_what_its_key_does() {
        use cosmic::widget::menu::Action as _;
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(crate::menu::MenuAct::Key(Action::Mkdir).message());
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    }

    #[test]
    fn window_close_cleans_up_like_exit() {
        // Alt+F4 / the title bar's ×: same temp cleanup as the Exit button and menu item.
        let app = app_with(Config::default(), State::default());
        assert!(matches!(
            app.on_close_requested(app.window_id()),
            Some(Message::Exit)
        ));
    }

    mod drawer_tests {
        use super::*;
        use crate::config::{AppTheme, LastTab};
        use crate::drawer::{Drawer, Setting};

        fn form(app: &App) -> crate::drawer::SettingsForm {
            match &app.drawer {
                Some(Drawer::Settings(f)) => f.clone(),
                d => panic!("no settings: {d:?}"),
            }
        }

        #[test]
        fn f1_toggles_help_and_about_replaces_it() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::Help));
            assert_eq!(app.drawer, Some(Drawer::Help));
            let _ = app.update(Message::Key(Action::About));
            assert_eq!(app.drawer, Some(Drawer::About));
            let _ = app.update(Message::Key(Action::About));
            assert_eq!(app.drawer, None);
        }

        #[test]
        fn donate_has_its_own_drawer() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::Donate));
            assert_eq!(app.drawer, Some(Drawer::Donate));
        }

        #[test]
        fn f9_copies_names_and_says_so() {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join("a.txt"), "").unwrap();
            let mut app = app_with(Config::default(), State::default());
            let entries = listing::scan(d.path(), false).unwrap();
            let panel = &mut app.panes[0].active_mut().panel;
            panel.set_listing(d.path().to_path_buf(), entries, Some("a.txt"));
            let _ = app.update(Message::Key(Action::CopyNames));
            assert!(
                app.msg().is_some_and(|m| m.contains("a.txt")),
                "{:?}",
                app.msg()
            );
        }

        #[test]
        fn messages_go_to_the_window_bar_not_the_panel() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::CompareLists)); // both panels empty: identical
            assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Info));
            assert_eq!(app.panes[0].active().error, None);
            let _ = app.update(Message::Key(Action::Down));
            assert_eq!(app.status, None); // gone with the next action
        }

        #[test]
        fn mount_result_keeps_a_newer_message() {
            let mut app = app_with(Config::default(), State::default());
            app.say(StatusKind::Busy, fl!("mounting"));
            app.say(StatusKind::Info, fl!("copied", n = 3)); // F9 while mounting
            let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/x"))));
            assert_eq!(app.msg(), Some(fl!("copied", n = 3).as_str()));
            let _ = app.update(Message::Unmounted("/media/x".into(), Ok(())));
            assert_eq!(app.msg(), Some(fl!("copied", n = 3).as_str()));
        }

        #[test]
        fn working_message_outlives_the_next_key() {
            let mut app = app_with(Config::default(), State::default());
            app.say(StatusKind::Busy, fl!("connecting"));
            let _ = app.update(Message::Key(Action::Down));
            assert_eq!(app.msg(), Some(fl!("connecting").as_str()));
            let _ = app.update(Message::Mounted(0, Err(mount::Error::WrongPassword)));
            assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
        }

        #[test]
        fn escape_closes_the_drawer_first() {
            let mut app = app_with(Config::default(), State::default());
            app.panes[0].active_mut().panel.set_filter(Some("x".into()));
            let _ = app.update(Message::Key(Action::Help));
            let _ = app.update(Message::DialogCancel);
            assert_eq!(app.drawer, None);
            assert!(app.panes[0].active().panel.filter().is_some()); // filter kept
        }

        #[test]
        fn settings_form_shows_config() {
            let config = Config {
                viewer: vec!["code".into(), "--wait".into()],
                home_dir: Some("/srv".into()),
                ..Config::default()
            };
            let mut app = app_with(config, State::default());
            let _ = app.update(Message::Key(Action::Settings));
            let f = form(&app);
            assert_eq!(
                (f.viewer.as_str(), f.editor.as_str(), f.home.as_str()),
                ("code --wait", "cosmic-edit", "/srv")
            );
        }

        #[test]
        fn settings_change_config() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::Settings));
            let set = |app: &mut App, s| {
                let _ = app.update(Message::Setting(s));
            };
            set(&mut app, Setting::ShowFkeys(false));
            set(&mut app, Setting::ShowHidden(true));
            set(&mut app, Setting::LastTabHome(true));
            set(&mut app, Setting::PackFormat(1));
            set(&mut app, Setting::Theme(2));
            set(&mut app, Setting::Language(1)); // "en": tests share the fallback loader
            set(&mut app, Setting::Viewer("code ".into()));
            set(&mut app, Setting::HomeDir("  ".into()));
            let c = &app.config;
            assert!(!c.show_fkeys && c.show_hidden);
            assert_eq!(c.last_tab_close, LastTab::Home);
            assert_eq!(c.pack_format, Format::PACK[1].ext());
            assert_eq!((c.app_theme, c.language.as_str()), (AppTheme::Dark, "en"));
            assert_eq!(c.viewer, ["code"]);
            assert_eq!(c.home_dir, None);
            assert_eq!(form(&app).viewer, "code "); // the space being typed stays
            assert!(app.panes[0].active().panel.show_hidden());
        }
    }

    mod mount_tests {
        use super::*;
        use shagoff_core::mount::{Error as MountError, Volume};

        fn drive(label: &str, path: &str) -> Drive {
            Drive {
                label: label.into(),
                path: path.into(),
            }
        }

        fn volume(name: &str, device: &str, mount: Option<&str>) -> Volume {
            Volume {
                name: name.into(),
                device: device.into(),
                mount: mount.map(String::from),
            }
        }

        fn list_len(app: &App) -> usize {
            match &app.dialog {
                Some(Dialog::List { items, .. }) => items.len(),
                _ => 0,
            }
        }

        #[test]
        fn drive_list_gets_unmounted_volumes_of_its_side_only() {
            let mut app = app_with(Config::default(), State::default());
            app.drives = vec![drive("/", "/"), drive("~", "/tmp")];
            let _ = app.update(Message::Key(Action::Drives(0)));
            let vols = vec![
                volume("sys", "/dev/nvme0n1p3", Some("file:///media/sys")),
                volume("STICK", "/dev/sda1", None),
            ];
            let _ = app.update(Message::Volumes(1, vols.clone())); // the other side's list
            assert_eq!(list_len(&app), 2);
            let _ = app.update(Message::Volumes(0, vols.clone()));
            assert_eq!(list_len(&app), 3); // only the unmounted one
            // Alt+F1, Esc, Alt+F1 before gio answered: two replies for one list (review).
            let _ = app.update(Message::Volumes(0, vols));
            assert_eq!(list_len(&app), 3);
            let Some(Dialog::List { items, .. }) = &app.dialog else {
                panic!("no list")
            };
            assert!(items[2].kind == Item::Mount && items[2].path == Path::new("/dev/sda1"));
        }

        #[test]
        fn unmounted_volumes_join_the_drive_list_and_a_pick_mounts() {
            let mut app = app_with(Config::default(), State::default());
            let vols = vec![
                volume("sys", "/dev/nvme0n1p3", Some("file:///media/sys")),
                volume("STICK", "/dev/sda1", None),
            ];
            let _ = app.update(Message::AllVolumes(vols));
            assert_eq!(app.volumes, [volume("STICK", "/dev/sda1", None)]);
            let _ = app.update(Message::MountVolume(0, "/dev/sda1".into()));
            assert_eq!(app.msg(), Some(fl!("mounting").as_str()));
        }

        #[test]
        fn volumes_after_list_closed_are_dropped() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Volumes(0, vec![volume("S", "/dev/sda1", None)]));
            assert!(app.dialog.is_none());
        }

        #[test]
        fn enter_on_volume_mounts_instead_of_going() {
            let mut app = app_with(Config::default(), State::default());
            app.drives = vec![drive("/", "/")];
            let _ = app.update(Message::Key(Action::Drives(0)));
            let _ = app.update(Message::Volumes(0, vec![volume("S", "/dev/sda1", None)]));
            app.panes[0].active_mut().pending = None;
            let _ = app.update(Message::ListPick(1));
            assert_eq!(pending_path(&app), None);
            assert_eq!(app.msg(), Some(fl!("mounting").as_str()));
        }

        #[test]
        fn mounted_goes_there_or_shows_error() {
            let mut app = app_with(Config::default(), State::default());
            app.panes[0].active_mut().pending = None;
            let _ = app.update(Message::Mounted(0, Err(MountError::WrongPassword)));
            assert_eq!(app.msg(), Some(fl!("mount-wrong-password").as_str()));
            app.say(StatusKind::Busy, fl!("connecting")); // the retry
            let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/x"))));
            assert_eq!(pending_path(&app), Some(PathBuf::from("/media/x")));
            assert_eq!(app.msg(), None);
        }

        #[test]
        fn ctrl_f_opens_connect_and_submit_reports_progress() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::Connect));
            assert!(matches!(app.dialog, Some(Dialog::Connect { .. })));
            let _ = app.update(Message::DialogInput("sftp://nas/".into()));
            let _ = app.update(Message::ConnectPassword("pw".into()));
            let Some(Dialog::Connect { url, password, .. }) = &app.dialog else {
                panic!("no dialog")
            };
            assert_eq!((url.as_str(), password.as_str()), ("sftp://nas/", "pw"));
            let _ = app.update(Message::DialogSubmit);
            // Stays open, as in TC: the outcome is shown in it.
            let note = |app: &App| match &app.dialog {
                Some(Dialog::Connect { note, .. }) => note.clone(),
                _ => panic!("dialog closed"),
            };
            assert_eq!(note(&app), Some((false, fl!("connecting"))));
            let bad = Err(MountError::Failed("gio: no such host".into()));
            let _ = app.update(Message::Connected(0, "sftp://nas/".into(), bad));
            assert_eq!(note(&app), Some((true, "gio: no such host".into())));
            assert!(app.connecting.is_none());
        }

        #[test]
        fn connect_remembers_the_address_and_esc_cancels() {
            let mut app = app_with(Config::default(), State::default());
            app.panes[0].active_mut().pending = None;
            let _ = app.update(Message::Key(Action::Connect));
            let _ = app.update(Message::DialogInput("sftp://bob:pw@nas/".into()));
            let _ = app.update(Message::DialogSubmit); // the gio task is never run in tests
            // A drive mounted meanwhile (Alt+F1) is not the address.
            let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/stick"))));
            assert!(app.config.connections.is_empty());
            let url = "sftp://bob:pw@nas/".to_string();
            let _ = app.update(Message::Connected(0, url, Ok(PathBuf::from("/run/gvfs/x"))));
            assert_eq!(app.config.connections, ["sftp://bob@nas/"]);
            // Saved addresses are offered; picking one fills the field.
            let _ = app.update(Message::Key(Action::Connect));
            let Some(Dialog::Connect { saved, .. }) = &app.dialog else {
                panic!("no dialog")
            };
            assert_eq!(saved, &["sftp://bob@nas/"]);
            let _ = app.update(Message::ConnectPick("smb://x/".into()));
            assert!(matches!(&app.dialog, Some(Dialog::Connect { url, .. }) if url == "smb://x/"));
            // Esc while connecting stops it.
            let _ = app.update(Message::DialogSubmit);
            let flag = app.connecting.as_ref().unwrap().1.clone();
            // A second submit meanwhile does nothing (its flag would replace this one).
            let _ = app.update(Message::DialogSubmit);
            assert!(Arc::ptr_eq(&flag, &app.connecting.as_ref().unwrap().1));
            let _ = app.update(Message::DialogCancel);
            assert!(flag.load(Ordering::Relaxed));
            let gone = Err(MountError::Cancelled);
            let _ = app.update(Message::Connected(0, "smb://x/".into(), gone));
            assert_eq!(app.msg(), Some(fl!("connect-cancelled").as_str()));
            assert_eq!(app.config.connections, ["sftp://bob@nas/"]); // not remembered
            assert!(app.connecting.is_none());
        }

        #[test]
        fn connect_forget_and_browse_results() {
            let config = Config {
                connections: vec!["smb://a/".into(), "smb://b/".into()],
                ..Config::default()
            };
            let mut app = app_with(config, State::default());
            let _ = app.update(Message::Key(Action::Connect));
            let _ = app.update(Message::ConnectForget("smb://a/".into()));
            assert_eq!(app.config.connections, ["smb://b/"]);
            let found = vec![("nas".to_string(), "sftp://nas.local/".to_string())];
            let _ = app.update(Message::Browsed(Ok(found.clone())));
            let Some(Dialog::Connect {
                saved, found: f, ..
            }) = &app.dialog
            else {
                panic!("no dialog")
            };
            assert_eq!(
                (saved.as_slice(), f),
                (&["smb://b/".to_string()][..], &found)
            );
        }

        #[test]
        fn disconnect_refuses_root_home_and_fixed_partitions() {
            let mut app = app_with(Config::default(), State::default());
            let cwd = app.panes[0].active().panel.cwd().to_path_buf();
            app.drives = vec![drive("/", "/"), drive("~", cwd.to_str().unwrap())];
            let _ = app.update(Message::Key(Action::Disconnect));
            assert_eq!(app.msg(), None);
            // A partition outside /media (like /home): udisks would ask for the admin password.
            app.drives = vec![
                drive("/", "/"),
                drive("~", "/x"),
                drive("tmp", cwd.to_str().unwrap()),
            ];
            let _ = app.update(Message::Key(Action::Disconnect));
            assert_eq!(app.msg(), None);
        }

        #[test]
        fn unmounted_sends_panes_inside_home() {
            let mut app = app_with(Config::default(), State::default());
            let cwd = app.panes[0].active().panel.cwd().to_path_buf();
            app.panes[0].active_mut().pending = None;
            let _ = app.update(Message::Unmounted("/nonexistent".into(), Ok(())));
            assert_eq!(pending_path(&app), None);
            let _ = app.update(Message::Unmounted(cwd, Ok(())));
            assert_eq!(pending_path(&app), Some(app.home.clone()));
            let failed = MountError::Failed("target is busy".into());
            let _ = app.update(Message::Unmounted("/media/x".into(), Err(failed)));
            assert_eq!(app.msg(), Some("target is busy"));
        }
    }

    mod cmdline {
        use super::*;

        /// A key no widget took (the panel has focus).
        fn typed(key: &str, code: Code, mods: Modifiers) -> Option<Message> {
            let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(key.into()),
                modified_key: Key::Character(key.into()),
                physical_key: Physical::Code(code),
                location: Location::Standard,
                modifiers: mods,
                text: None,
                repeat: false,
            });
            route_event(
                event,
                event::Status::Ignored,
                cosmic::iced::window::Id::unique(),
            )
        }

        fn at(dir: &Path) -> App {
            let mut app = app_with(Config::default(), State::default());
            listed_at(&mut app, 0, dir);
            app
        }

        #[test]
        fn letters_go_to_the_command_line_space_only_when_it_has_text() {
            assert!(matches!(
                typed("l", Code::KeyL, Modifiers::empty()),
                Some(Message::CmdType('l'))
            ));
            assert!(matches!(
                typed("L", Code::KeyL, Modifiers::SHIFT),
                Some(Message::CmdType('L'))
            ));
            assert!(typed("l", Code::KeyL, Modifiers::CTRL).is_none());
            let tmp = tempfile::tempdir().unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::Key(Action::Mark)); // empty: Space marks
            assert_eq!(app.cmdline, "");
            let _ = app.update(Message::CmdType('l'));
            let _ = app.update(Message::Key(Action::Mark));
            let _ = app.update(Message::CmdType('s'));
            assert_eq!(app.cmdline, "l s");
        }

        #[test]
        fn hidden_command_line_takes_nothing() {
            let config = Config {
                show_cmdline: false,
                ..Config::default()
            };
            let mut app = app_with(config, State::default());
            let _ = app.update(Message::CmdType('l'));
            let _ = app.update(Message::Key(Action::CmdCwd));
            assert_eq!(app.cmdline, "");
        }

        #[test]
        fn cd_changes_the_panel_dir_and_is_remembered() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir(tmp.path().join("sub")).unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::CmdInput("cd sub".into()));
            let _ = app.update(Message::CmdEnter);
            assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
            assert_eq!(app.cmdline, "");
            assert_eq!(app.commands, ["cd sub"]);
        }

        #[test]
        fn enter_in_the_panel_runs_a_typed_line() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir(tmp.path().join("sub")).unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::CmdInput("cd sub".into()));
            let _ = app.update(Message::Key(Action::Enter));
            assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
        }

        #[test]
        fn a_command_runs_in_the_panel_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::CmdInput("touch made".into()));
            let _ = app.update(Message::CmdEnter);
            let made = tmp.path().join("made");
            for _ in 0..100 {
                if made.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert!(made.exists());
            assert_eq!(app.msg(), None);
        }

        #[test]
        fn ctrl_enter_is_for_the_name_not_a_submit() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("a b.txt"), "").unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::Key(Action::Down)); // off ".."
            let _ = app.update(Message::CmdInput("cat".into()));
            let _ = app.update(Message::Modifiers(Modifiers::CTRL));
            let _ = app.update(Message::CmdEnter); // the field reports Ctrl+Enter as Enter
            assert_eq!(app.cmdline, "cat 'a b.txt'");
            let _ = app.update(Message::Modifiers(Modifiers::CTRL | Modifiers::SHIFT));
            let _ = app.update(Message::CmdEnter);
            let full = tmp.path().join("a b.txt").display().to_string();
            assert_eq!(app.cmdline, format!("cat 'a b.txt' '{full}'"));
        }

        #[test]
        fn parent_row_inserts_nothing() {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::Key(Action::CmdName));
            assert_eq!(app.cmdline, "");
        }

        #[test]
        fn ctrl_e_and_alt_f8_bring_back_commands() {
            let tmp = tempfile::tempdir().unwrap();
            let state = State {
                commands: vec!["make".into(), "ls".into()],
                ..State::default()
            };
            let mut app = app_with(Config::default(), state);
            listed_at(&mut app, 0, tmp.path());
            let _ = app.update(Message::Key(Action::CmdPrevious));
            assert_eq!(app.cmdline, "make");
            let _ = app.update(Message::FieldKey(Action::CmdPrevious));
            assert_eq!(app.cmdline, "ls");
            let _ = app.update(Message::FieldKey(Action::CmdHistory));
            let _ = app.update(Message::Key(Action::Down));
            let _ = app.update(Message::Key(Action::Enter));
            assert!(app.dialog.is_none());
            assert_eq!(app.cmdline, "ls");
            let _ = app.update(Message::ListPick(0));
            assert_eq!(app.cmdline, "ls"); // no dialog: nothing picked
        }

        #[test]
        fn regression_viewer_letters_start_the_line_when_no_viewer() {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = at(tmp.path());
            for (k, code) in [
                ("n", Code::KeyN),
                ("т", Code::KeyN),
                ("p", Code::KeyP),
                ("1", Code::Digit1),
            ] {
                if let Some(m) = typed(k, code, Modifiers::empty()) {
                    let _ = app.update(m);
                }
            }
            assert_eq!(app.cmdline, "nтp1");
        }

        #[test]
        fn regression_double_click_and_quick_search_open_not_run() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir(tmp.path().join("sub")).unwrap();
            std::fs::create_dir(tmp.path().join("sub/in")).unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::CmdInput("cd /".into()));
            let _ = app.update(Message::DoubleClick(0, 1));
            assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
            listed_at(&mut app, 0, &tmp.path().join("sub"));
            let _ = app.update(Message::Key(Action::QuickSearch('i')));
            let _ = app.update(Message::SearchSubmit);
            assert_eq!(app.panes[0].active().target(), tmp.path().join("sub/in"));
            assert_eq!(app.cmdline, "cd /");
        }

        #[test]
        fn regression_terminal_line_with_a_comment_keeps_the_shell() {
            let argv = shagoff_core::cmdline::argv("ls # x", Path::new("/d"), Some(&[]));
            assert!(argv[4].ends_with("\nexec \"${SHELL:-sh}\""), "{argv:?}");
        }

        #[test]
        fn shift_space_does_not_start_the_line_with_a_space() {
            let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(" ".into()),
                modified_key: Key::Character(" ".into()),
                physical_key: Physical::Code(Code::Space),
                location: Location::Standard,
                modifiers: Modifiers::SHIFT,
                text: None,
                repeat: false,
            });
            let m = route_event(
                event,
                event::Status::Ignored,
                cosmic::iced::window::Id::unique(),
            );
            let mut app = app_with(Config::default(), State::default());
            if let Some(m) = m {
                let _ = app.update(m);
            }
            assert_eq!(app.cmdline, "");
        }

        #[test]
        fn escape_with_a_hidden_line_clears_the_filter() {
            let tmp = tempfile::tempdir().unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::CmdInput("x".into()));
            app.config.show_cmdline = false;
            app.panes[0].active_mut().panel.set_filter(Some("a".into()));
            let _ = app.update(Message::DialogCancel);
            assert_eq!(app.panes[0].active().panel.filter(), None);
        }

        #[test]
        fn escape_clears_the_line() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::CmdInput("rm -rf x".into()));
            let _ = app.update(Message::DialogCancel);
            assert_eq!(app.cmdline, "");
        }

        #[test]
        fn f_keys_from_the_command_line_reach_the_panel() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("f"), "").unwrap();
            let mut app = at(tmp.path());
            let _ = app.update(Message::Key(Action::Down));
            let _ = app.update(Message::CmdInput("x".into()));
            let _ = app.update(Message::FieldKey(Action::Copy));
            assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
        }
    }

    mod tabs_more {
        use super::*;

        /// tmp with dirs `a`, `a/in` and file `f`; pane 0 shows tmp, cursor on `a`.
        fn setup() -> (tempfile::TempDir, App) {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join("a/in")).unwrap();
            std::fs::write(tmp.path().join("f"), "").unwrap();
            let mut app = app_with(Config::default(), State::default());
            listed_at(&mut app, 0, tmp.path());
            let _ = app.update(Message::Key(Action::Down));
            (tmp, app)
        }

        fn key(app: &mut App, a: Action) {
            let _ = app.update(Message::Key(a));
        }

        #[test]
        fn ctrl_up_opens_the_dir_under_the_cursor_in_a_new_tab() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::TabOpen);
            assert_eq!(
                cwds_or_targets(&app, 0),
                [tmp.path().into(), tmp.path().join("a")]
            );
            assert_eq!(app.panes[0].active_index(), 1);
            // On "..": the parent; on a file: the dir itself.
            let (tmp2, mut app) = setup();
            key(&mut app, Action::Up);
            key(&mut app, Action::TabOpen);
            let parent = tmp2.path().parent().unwrap().to_path_buf();
            assert_eq!(app.panes[0].active().target(), parent);
            let (tmp3, mut app) = setup();
            key(&mut app, Action::End);
            key(&mut app, Action::TabOpen);
            assert_eq!(app.panes[0].active().target(), tmp3.path());
        }

        fn cwds_or_targets(app: &App, side: usize) -> Vec<PathBuf> {
            app.panes[side].items().iter().map(Tab::target).collect()
        }

        #[test]
        fn ctrl_shift_up_opens_it_in_the_other_panel_and_keeps_focus() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::TabOpenOther);
            assert_eq!(app.panes[1].items().len(), 2);
            assert_eq!(app.panes[1].active().target(), tmp.path().join("a"));
            assert_eq!(app.panes[0].items().len(), 1);
            assert_eq!(app.active, 0);
        }

        #[test]
        fn copy_and_move_a_tab_to_the_other_panel() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::TabCopyOther);
            assert_eq!(app.panes[0].items().len(), 1);
            assert_eq!(app.panes[1].items().len(), 2);
            assert_eq!(app.panes[1].active().target(), tmp.path());
            assert_eq!(app.active, 1);
            // The last tab of a panel stays.
            key(&mut app, Action::SwitchPane);
            key(&mut app, Action::TabMoveOther);
            assert_eq!(app.panes[0].items().len(), 1);
            assert!(app.msg().is_some());
            // Pane 1 has two: one moves over, with its lock and name.
            key(&mut app, Action::SwitchPane);
            key(&mut app, Action::TabLock);
            let id = app.panes[1].active().id;
            key(&mut app, Action::TabMoveOther);
            assert_eq!(app.panes[1].items().len(), 1);
            assert_eq!(app.panes[0].items().len(), 2);
            assert_eq!(app.panes[0].active().id, id);
            assert!(app.panes[0].active().locked);
            assert_eq!(app.active, 0);
        }

        #[test]
        fn locked_tab_stays_and_navigation_opens_a_new_one() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::NewTab);
            key(&mut app, Action::TabLock);
            assert!(app.panes[0].active().locked);
            key(&mut app, Action::CloseTab);
            assert_eq!(app.panes[0].items().len(), 2);
            assert!(app.msg().is_some());
            let i = app.panes[0].active_index();
            let _ = app.update(Message::CloseTabAt(0, i));
            assert_eq!(app.panes[0].items().len(), 2);
            key(&mut app, Action::Enter); // cursor on `a`
            assert_eq!(app.panes[0].items().len(), 3);
            assert_eq!(app.panes[0].active().target(), tmp.path().join("a"));
            assert!(!app.panes[0].active().locked);
            let locked = &app.panes[0].items()[1];
            assert!(locked.locked);
            assert_eq!(locked.target(), tmp.path());
            // Unlock: a plain tab again.
            app.panes[0].select(1);
            key(&mut app, Action::TabLock);
            assert!(!app.panes[0].active().locked);
        }

        #[test]
        fn rename_and_reset_a_tab_caption() {
            let (_tmp, mut app) = setup();
            key(&mut app, Action::TabRename);
            let shown = format::dir_title(app.panes[0].active().panel.cwd());
            assert!(matches!(&app.dialog, Some(Dialog::Input { input, .. }) if *input == shown));
            let _ = app.update(Message::DialogInput("Work".into()));
            let _ = app.update(Message::DialogSubmit);
            assert_eq!(app.panes[0].active().name.as_deref(), Some("Work"));
            key(&mut app, Action::TabRename); // the own caption is offered
            assert!(matches!(&app.dialog, Some(Dialog::Input { input, .. }) if input == "Work"));
            let _ = app.update(Message::DialogInput("  ".into()));
            let _ = app.update(Message::DialogSubmit);
            assert_eq!(app.panes[0].active().name, None);
        }

        #[test]
        fn close_others_keeps_active_and_locked() {
            let (_tmp, mut app) = setup();
            key(&mut app, Action::NewTab);
            key(&mut app, Action::TabLock);
            let locked = app.panes[0].active().id;
            key(&mut app, Action::NewTab);
            key(&mut app, Action::NewTab);
            let active = app.panes[0].active().id;
            key(&mut app, Action::CloseOtherTabs);
            let ids: Vec<u64> = app.panes[0].items().iter().map(|t| t.id).collect();
            assert_eq!(ids, [locked, active]);
            assert_eq!(app.panes[0].active().id, active);
        }

        #[test]
        fn regression_find_feed_leaves_a_locked_tab_alone() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::TabLock);
            key(&mut app, Action::FindFiles);
            let _ = app.update(Message::FindStart);
            let id = find_dialog(&mut app).id;
            let found = vec![tmp.path().join("f")];
            let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, found)));
            let _ = app.update(Message::FindFeed);
            assert_eq!(app.panes[0].items().len(), 2);
            assert!(app.panes[0].items()[0].results.is_none());
            assert!(app.panes[0].active().results.is_some());
        }

        #[test]
        fn regression_scan_in_flight_does_not_move_a_tab_locked_meanwhile() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::Enter); // into `a`, not listed yet
            key(&mut app, Action::TabLock);
            let t = app.panes[0].active();
            assert!(t.pending.is_none());
            assert_eq!(t.target(), tmp.path());
        }

        #[test]
        fn regression_history_step_keeps_the_locked_tabs_place() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::Enter);
            listed_at(&mut app, 0, &tmp.path().join("a"));
            key(&mut app, Action::TabLock);
            key(&mut app, Action::HistoryBack);
            assert_eq!(app.panes[0].items().len(), 2);
            assert_eq!(app.panes[0].active().target(), tmp.path());
            let locked = &app.panes[0].items()[0];
            assert_eq!(
                locked.history.clone().back(),
                Some(tmp.path().to_path_buf())
            );
        }

        #[test]
        fn regression_startup_path_does_not_repoint_a_locked_tab() {
            let tmp = tempfile::tempdir().unwrap();
            let state = State {
                panes: [
                    PaneState {
                        tabs: vec!["/".into()],
                        active: 0,
                        locked: vec![true],
                        ..PaneState::default()
                    },
                    PaneState::default(),
                ],
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
            let tabs: Vec<PathBuf> = app.panes[0].items().iter().map(Tab::target).collect();
            assert_eq!(
                tabs,
                [PathBuf::from("/"), tmp.path().canonicalize().unwrap()]
            );
            assert!(app.panes[0].items()[0].locked);
            assert_eq!(app.panes[0].active_index(), 1);
        }

        #[test]
        fn brief_view_survives_a_restart() {
            let (_tmp, mut app) = setup();
            key(&mut app, Action::NewTab);
            key(&mut app, Action::ViewBrief);
            app.save_state();
            let state = app.saved.clone();
            assert_eq!(state.panes[0].brief, [false, true]);
            let app = app_with(Config::default(), state);
            let brief: Vec<bool> = app.panes[0].items().iter().map(|t| t.brief).collect();
            assert_eq!(brief, [false, true]);
        }

        #[test]
        fn lock_and_caption_survive_a_restart() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::NewTab);
            key(&mut app, Action::TabLock);
            key(&mut app, Action::TabRename);
            let _ = app.update(Message::DialogInput("W".into()));
            let _ = app.update(Message::DialogSubmit);
            app.save_state();
            let state = app.saved.clone();
            assert_eq!(state.panes[0].locked, [false, true]);
            assert_eq!(state.panes[0].names, ["", "W"]);
            let app = app_with(Config::default(), state);
            let t = &app.panes[0].items()[1];
            assert!(t.locked);
            assert_eq!(t.name.as_deref(), Some("W"));
            assert_eq!(t.target(), tmp.path());
            assert!(!app.panes[0].items()[0].locked);
        }
    }
}
