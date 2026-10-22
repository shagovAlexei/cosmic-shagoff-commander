use crate::clip;
use crate::config::{self, Config, HotEntry, LastTab, State, ToolButton};
use crate::dialogs::{
    self, Dialog, FindField, InputOp, Item, ListItem, ListKind, MaskOp, MrField, SyncOpt, Toggle,
};
use crate::drawer::{self, Drawer, Setting, SettingsForm};
use crate::fl;
use crate::hotlist::{self, HotEdit, HotMsg};
use crate::jobs::{self, Job};
use crate::keymap::{self, Action, ListerKey};
use crate::lister::{self, Lister, Slide};
use alacritty_terminal::event::Event as TermEvent;
use cosmic::app::{Core, Task};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::clipboard::dnd::{DndEvent, OfferEvent};
use cosmic::iced::keyboard::Modifiers;
use cosmic::iced::widget::scrollable::{self, AbsoluteOffset};
use cosmic::iced::widget::text_editor;
use cosmic::iced::{Size, Subscription, event, keyboard, mouse};
use cosmic::{Application, Element, widget};
use shagoff_core::archive::{self, Format};
use shagoff_core::checksum::{self, Algo, Check};
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
use shagoff_core::openwith;
use shagoff_core::ops::{self, ErrorChoice, Method, PlanError, Report, Resolution};
use shagoff_core::panel::{self, PARENT, Panel, Show};
use shagoff_core::repack::Change;
use shagoff_core::session::{self, PaneState};
use shagoff_core::tabs::Tabs;
use shagoff_core::terminals::{self, Terminal};
use shagoff_core::viewport;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

pub const APP_ID: &str = "io.github.shagovAlexei.cosmic-shagoff-commander";
// ponytail: list height is guessed until the scrollable reports its bounds (it does on the first event).
const FALLBACK_LIST_H: f32 = 400.0;
const FALLBACK_LIST_W: f32 = 500.0;
/// Rows sorted on the UI thread at most (Ctrl+F3…F6); more go through a background read.
const SORT_HERE_MAX: usize = 20_000;
/// How long Enter waits for the rows of the dir being entered (`Tab::pending_at`).
const ENTER_WAIT: Duration = Duration::from_secs(1);

/// A right press while the right button marks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    /// Its number (`App::right_presses`): a late timer of an earlier press is dropped.
    pub n: u64,
    pub side: usize,
    /// The tab pressed in, and the row (`None`: empty space).
    pub tab: u64,
    pub row: Option<usize>,
    /// Held long enough: the release opens the menu.
    pub armed: bool,
    /// Dragged onto other rows (TC): the mark they all get and the last row reached. The release
    /// then neither toggles nor opens the menu.
    pub drag: Option<(bool, usize)>,
}

/// The viewer window's title bar.
#[derive(Clone, Copy, Debug)]
pub enum ListerWin {
    Drag,
    Maximize,
}

pub struct Flags {
    pub left: Option<PathBuf>,
}

/// Ctrl+F8's tree on a tab.
#[derive(Debug)]
pub struct PanelTree {
    /// Its reads' id (`Message::TreeFilled`).
    pub id: u64,
    pub tree: shagoff_core::tree::Tree,
    /// The file list's scroll, back when the tree goes.
    files_offset: f32,
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
    /// Ctrl+Shift+F1 Thumbnails view (cells with pictures); never together with `brief`.
    pub thumbs: bool,
    /// Ctrl+F8 (TC): the dirs as a tree instead of the files; the other panel shows the dir under
    /// its cursor.
    pub tree: Option<PanelTree>,
    /// Brief view: first column on screen.
    pub col: usize,
    /// Brief view: touchpad scroll not yet worth a column, px.
    wheel: f32,
    /// Generation and path of the scan in flight; any other result is stale.
    pub(crate) pending: Option<(u64, PathBuf)>,
    /// When that scan started: Enter waits for a step's rows only so long (a hung read).
    pending_at: Instant,
    pub error: Option<String>,
    /// Alt+← / Alt+→ / Alt+↓.
    pub(crate) history: History,
    /// Alt+F7 "To panel": (dir searched, found paths) listed instead of the dir.
    pub(crate) results: Option<(PathBuf, Arc<Vec<PathBuf>>)>,
    /// The results are Ctrl+B's branch (every file below), not a search: Ctrl+B again leaves.
    pub(crate) branch: bool,
    /// Ctrl+B being read: its request id; any dir change (`App::load`) drops it.
    pub(crate) branching: Option<u64>,
    /// TC locked tab: cannot be closed, leaving its dir opens a new tab.
    pub locked: bool,
    /// TC «locked, dir changes allowed» (160): its dir. It moves about, but is read there again
    /// once another tab is shown, and opens there next time; cannot be closed.
    pub home: Option<PathBuf>,
    /// Own caption instead of the dir's name.
    pub name: Option<String>,
    /// Stops the dir size counts (Space, Alt+Shift+Enter) of the dir shown; set when it is left.
    counting: Arc<AtomicBool>,
    /// Stops the Ctrl+B read under way (Esc, the dir left).
    branch_stop: Arc<AtomicBool>,
}

/// A closed tab's background reads (branch, dir sizes) stop with it.
impl Drop for Tab {
    fn drop(&mut self) {
        self.counting.store(true, Ordering::Relaxed);
        self.branch_stop.store(true, Ordering::Relaxed);
    }
}

impl Tab {
    /// Drop the Ctrl+B read under way, if any: its result is no longer wanted.
    fn stop_branch(&mut self) {
        self.branching = None;
        self.branch_stop.store(true, Ordering::Relaxed);
    }

    fn new(id: u64, cwd: PathBuf) -> Self {
        Self {
            id,
            panel: Panel::new(cwd),
            offset: 0.0,
            height: FALLBACK_LIST_H,
            width: FALLBACK_LIST_W,
            brief: false,
            thumbs: false,
            tree: None,
            col: 0,
            wheel: 0.0,
            pending: None,
            pending_at: Instant::now(),
            error: None,
            history: History::default(),
            results: None,
            branch: false,
            branching: None,
            locked: false,
            home: None,
            name: None,
            counting: Arc::default(),
            branch_stop: Arc::default(),
        }
    }

    /// Tab caption: own name or the dir's, `*` in front when locked (as in TC); `!` when one
    /// locked with dir changes allowed is away from its dir.
    pub fn title(&self) -> String {
        let name = (self.name.clone()).unwrap_or_else(|| format::dir_title(self.panel.cwd()));
        match &self.home {
            Some(h) if h != self.panel.cwd() => format!("!{name}"),
            Some(_) => format!("*{name}"),
            None if self.locked => format!("*{name}"),
            None => name,
        }
    }

    /// Not to be closed (Ctrl+W, Ctrl+Shift+W, the middle button).
    pub fn pinned(&self) -> bool {
        self.locked || self.home.is_some()
    }

    pub fn lock(&self) -> crate::menu::Lock {
        crate::menu::Lock {
            locked: self.locked,
            roam: self.home.is_some(),
        }
    }

    /// Brief view: (rows per column, columns on screen) for rows `row_h` px high.
    /// `scale`: the panel text size over the default.
    pub fn brief_grid(&self, row_h: f32, scale: f32) -> (usize, usize) {
        let longest = self.panel.longest_name();
        (
            viewport::brief_rows(row_h, self.height),
            viewport::brief_cols(self.width, viewport::brief_col_w(longest, scale)),
        )
    }

    /// Thumbnails view: cells per row.
    pub fn thumb_cols(&self) -> usize {
        viewport::brief_cols(self.width, crate::thumbs::CELL_W)
    }

    /// Where the tab is going: the dir being scanned, else the shown one. Rescans use this so they
    /// never cancel a navigation still in flight.
    pub(crate) fn target(&self) -> PathBuf {
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
            thumbs: self.thumbs,
            tree: None, // a new tab shows the files
            col: self.col,
            wheel: 0.0,
            pending: None,
            pending_at: Instant::now(),
            error: None,
            history: self.history.clone(),
            results: self.results.clone(),
            branch: self.branch,
            branching: None,
            locked: false,
            home: None,
            name: None,
            counting: Arc::default(),
            branch_stop: Arc::default(),
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

/// A header edge being dragged: (side, edge 1..=4, first pointer x and the width then).
pub type ColDrag = (usize, usize, Option<(f32, f32)>);

/// A file operation in progress (the progress dialog's data).
pub struct Running {
    pub side: usize,
    pub kind: OpKind,
    pub done: u64,
    pub total: u64,
    pub current: String,
    control: jobs::Control,
    /// Name to put the source pane's cursor on afterwards (rename).
    focus: Option<String>,
    /// Enter / F3 in an archive: what to do with the extracted file once the job succeeds.
    open: Option<(Open, PathBuf)>,
    /// "In background": no dialog, the panels work; progress shows above the status line.
    pub hidden: bool,
}

/// An operation started while another one runs: it waits its turn, then runs in the background.
pub struct Queued {
    side: usize,
    pub kind: OpKind,
    job: Job,
    focus: Option<String>,
    /// F5 / F6: sources and target, planned again when it starts (the running one may have
    /// made the target dir meanwhile).
    replan: Option<(Vec<PathBuf>, PathBuf)>,
}

impl Queued {
    pub fn label(&self) -> String {
        jobs::describe(&self.job)
    }
}

impl Running {
    pub fn paused(&self) -> bool {
        self.control.pause.load(Ordering::Relaxed)
    }
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
    /// An archive inside an archive: in the panel, as a dir; Backspace at its root goes back to
    /// this dir of the outer one.
    Browse(PathBuf),
}

/// What to do with a file extracted from an archive.
enum How {
    /// Program from the config (empty → the default).
    Run(Vec<String>, &'static [&'static str]),
    /// F4: the editor, and watch the file.
    Edit(Vec<String>),
    Lister,
    /// Enter on an archive inside an archive: open it in the panel.
    Browse,
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
    /// A toolbar button held with Shift, moved to where the pointer goes.
    pub tool_drag: shagoff_core::reorder::Reorder,
    dialog: Option<Dialog>,
    /// Help / about / settings in the side drawer.
    pub(crate) drawer: Option<Drawer>,
    /// Built once (the widget borrows it), rebuilt when the language changes. Two parts, so
    /// «Check for updates» sits under the version: [icon … version, links … comments].
    pub(crate) about: [cosmic::widget::about::About; 2],
    /// About → «Check for updates»: `None` not asked since About opened.
    pub(crate) update_check: Option<shagoff_core::update::Status>,
    pub(crate) job: Option<Running>,
    /// TC's background queue: started one after another once `job` ends.
    pub(crate) queue: VecDeque<Queued>,
    /// A background job's question while another dialog is open: shown once that one closes.
    held: Option<Dialog>,
    /// `config.colors`, parsed (`Scheme::new` on every change of them).
    pub(crate) scheme: shagoff_core::colors::Scheme,
    /// `config.panel_font` as a font (built when it changes).
    face: cosmic::iced::Font,
    /// Files of the right-button drag in flight (`Message::DragStart`).
    right_drag: Option<Arc<Vec<PathBuf>>>,
    /// A right press while the right button marks (`Config.right_marks`).
    pub right_hold: Option<Hold>,
    /// Numbers the right presses: a late `RightHeld` of an earlier one is dropped.
    right_presses: u64,
    /// One of our rows is being dragged: Esc cancels it.
    dragging: bool,
    /// Quit confirmed while a job ran: exit when it has stopped.
    quitting: bool,
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
    /// Ctrl+Shift+S options as last compared (written to the state).
    sync: config::SyncPrefs,
    /// The left pane's share of the width (`viewport::split_or_half`), and whether the divider
    /// is being dragged.
    pub(crate) split: f32,
    splitting: bool,
    /// Where the pointer was when the held divider first moved: a click without a drag (a pixel
    /// or two of jitter) leaves the split alone.
    split_from: Option<f32>,
    win_w: f32,
    pub drives: Vec<Drive>,
    /// Installed terminals and the system's default among them (`terminals::scan`, at start).
    pub terminals: Vec<Terminal>,
    pub term_default: Option<usize>,
    /// The built-in terminal's event channel (`term::subscription`, while it is on).
    term_tx: Option<tokio::sync::mpsc::UnboundedSender<TermEvent>>,
    /// The built-in terminal, once started; `None` after its shell ended (started again on Ctrl+O
    /// or a click).
    pub term: Option<Box<crate::term::Pane>>,
    /// Ctrl+O: the terminal in place of the panels.
    pub term_full: bool,
    pub term_id: widget::Id,
    /// Its shell ended: not started again on its own (Ctrl+O or a click does).
    term_ended: bool,
    /// Its top edge held: where the pointer was pressed (y) and the rows then; the rows now.
    term_drag: Option<(f32, u16)>,
    pub term_rows: u16,
    /// A pane was clicked: the terminal gives the keys up (`update`'s tail).
    term_blur: bool,
    /// The terminal has the keys (`TermFocused`).
    term_focused: bool,
    /// Turned on by Ctrl+`: focused once its shell starts.
    term_want_focus: bool,
    /// The gvfs mounts last read (`Message::Gvfs`), appended to `drives`.
    gvfs: Vec<Drive>,
    /// The gvfs dir is to be read again / is being read: in the background, one read at a time —
    /// it blocks while any of its network mounts hangs.
    gvfs_want: bool,
    gvfs_busy: bool,
    /// Column widths as shown: the config's, or the one being dragged (written on release).
    pub cols: [f32; 4],
    /// A header edge being dragged: (side, edge 1..=4, first pointer x and the width then).
    pub col_drag: Option<ColDrag>,
    /// A tab held by the mouse: (side, its index now). Moved over the tab the pointer enters.
    pub tab_drag: Option<(usize, usize)>,
    /// The pane the pointer is over while a tab is held (released there, it goes to it).
    pane_hover: Option<usize>,
    /// The dir row files are being dragged over: (side, row).
    pub drop_hover: Option<(usize, usize)>,
    /// Thumbnails view: path → (its mtime then, the picture; `None` while made or unreadable).
    pub thumbs: HashMap<PathBuf, (SystemTime, Option<widget::image::Handle>)>,
    /// Pictures being made now.
    thumbs_busy: usize,
    /// Where held files were last seen (window coordinates).
    drop_at: Option<cosmic::iced::Point>,
    /// Ctrl+D: the submenus opened, outermost first (their indices in `config.hotlist`).
    hot_sub: Vec<usize>,
    /// Volumes not mounted yet (a stick just plugged in): listed with the drives, a pick mounts.
    pub volumes: Vec<mount::Volume>,
    /// When `volumes` was last asked for: gio runs at most every few seconds.
    volumes_at: Option<std::time::Instant>,
    /// (free, total) bytes of each pane's current disk.
    pub space: [Option<(u64, u64)>; 2],
    /// Status line: the last message (in the active pane's half).
    pub status: Option<Status>,
    /// "Open with…" list: the files to open, as they were when it was asked for.
    open_with: Vec<PathBuf>,
    /// Their type, for "Default".
    open_with_mime: String,
    /// Ctrl+Q: id of the newest read; an older one waiting out its delay is skipped.
    quick_latest: Arc<std::sync::atomic::AtomicU64>,
    /// Ctrl+Q on a dir: stops its count when the cursor moves on.
    quick_stop: Arc<AtomicBool>,
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
    /// Archives opened from inside another (155): the copy in the temp dir → the dir in the outer
    /// archive it came from (Backspace at its root goes back there).
    nested: HashMap<PathBuf, PathBuf>,
    /// Each pane's shown tab after the last `update`: one that stops being shown and is locked
    /// with dir changes allowed goes back to its dir (`roam_back`).
    shown_tabs: [u64; 2],
    /// The edited file being put back («Обновить» answered): if that fails, it is asked again.
    updating: Option<PathBuf>,
    /// Ctrl+F in progress: (address, cancel flag set by Esc).
    pub(crate) connecting: Option<(String, Arc<AtomicBool>)>,
    /// Command line under the panels (TC `path>`): its text, field id, history (last first).
    pub cmdline: String,
    pub(crate) cmd_id: widget::Id,
    pub commands: Vec<String>,
    /// Num+ / Num− masks, last first: the dialog opens with the last, ↑ / ↓ go through them.
    pub masks: Vec<String>,
    /// "Open with…" → "Other program…" commands, last first; ↑ / ↓ as for masks.
    pub other_cmds: Vec<String>,
    /// Alt+Shift+F9 under way: another waits for it.
    testing: bool,
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
    /// Ctrl+F8: a click on row i of side's tree (the other panel follows) / a double click
    /// (open / close it).
    TreeRow(usize, usize),
    TreeRowToggle(usize, usize),
    /// Alt+F10: a click on row i (on the row under the cursor: open / close it).
    TreePick(usize),
    /// Alt+F10: for the tree dialog with this id, dirs read — each with its subdirs — and the one
    /// to put the cursor on.
    TreeFilled(u64, Vec<(PathBuf, Vec<OsString>)>, Option<PathBuf>),
    /// Alt+Shift+F9 done: each archive and the files read from it, or what is wrong with it.
    Tested(Vec<(PathBuf, Result<u64, String>)>),
    /// Ctrl+L counted: the targets' occupied space (`None`: could not / stopped).
    Occupied(Option<shagoff_core::props::Usage>),
    /// Ctrl+Shift+F5 done (`App::make_links`) for tab id: the sources linked, and the error to
    /// show (one line for every link that failed), or the error that stopped it all.
    Linked(u64, Result<(Vec<PathBuf>, Option<String>), String>),
    /// F7 done (`App::mkdir`): the dir it was typed in, the name as typed and its first part.
    Made {
        side: usize,
        cwd: PathBuf,
        name: String,
        first: Option<String>,
        made: Result<(), String>,
    },
    /// The built-in terminal's event channel is up: its shell starts.
    TermTx(tokio::sync::mpsc::UnboundedSender<TermEvent>),
    TermEvent(TermEvent),
    /// A click on the ended terminal's line: start its shell again.
    TermStart,
    /// The terminal gained / lost the keys.
    TermFocused(bool),
    /// The terminal's top edge: pressed, the pointer at y (window), let go.
    TermDragStart,
    TermDragAt(f32),
    TermDragEnd,
    /// The installed terminals and the default's index (`terminals::scan`).
    Terminals(Vec<Terminal>, Option<usize>),
    /// The gvfs mounts, read in the background (`App::read_gvfs`).
    Gvfs(Vec<Drive>),
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
    /// The right press with this number was held long enough: its release opens the menu.
    RightHeld(u64),
    /// The right button came up, anywhere.
    RightUp,
    /// The pointer onto row `i` of a pane while the right button is held there (`right_hold`).
    RowEnter(usize, usize),
    /// A button of one pane (column header, `\\`, `..`): that pane becomes active and acts.
    PaneKey(usize, Action),
    /// side, scroll offset y, viewport height (of the active tab)
    Scrolled(usize, f32, f32),
    /// side, real size of the pane's list (from a sensor: on_scroll misses resizes)
    Resized(usize, Size),
    /// side, mouse wheel over a Brief list: one column per notch
    BriefWheel(usize, mouse::ScrollDelta),
    /// A picture made for the Thumbnails view: path, its mtime when asked, the picture.
    Thumb(PathBuf, SystemTime, Option<widget::image::Handle>),
    SelectTab(usize, usize),
    /// Left press on a tab: select it and start dragging it.
    TabPress(usize, usize),
    /// The pointer entered a tab (moves a dragged tab there).
    TabEnter(usize, usize),
    /// The pointer's x in a window while a tab is held.
    TabDragAt(cosmic::iced::window::Id, f32),
    CloseTabAt(usize, usize),
    Modifiers(Modifiers),
    DialogInput(String),
    DialogSubmit,
    /// F2 «В очередь» in the F5 / F6 dialog: run it in the background (after the running one).
    DialogQueue,
    DialogCancel,
    /// Escape in a window: the main one cancels the dialog; a popup (context menu) closes,
    /// since the menu widget only sees the main window's keys.
    Escape(cosmic::iced::window::Id),
    MrInput(MrField, String),
    MrCase(Case),
    MrHelp,
    /// A mask token button: appended to the name mask, which keeps the focus.
    MrInsert(&'static str),
    /// Pack dialog: format button.
    PackFormat(Format),
    SumAlgo(Algo),
    /// Checksum files written (tab, the files) or the error.
    Summed(u64, Result<Vec<PathBuf>, String>),
    /// Each checksum file verified: its (file, result) rows, or why it could not be read.
    Verified(Vec<(PathBuf, Result<checksum::Report, String>)>),
    /// Alt+F5's password field (`true`: the second one).
    PackPassword(bool, String),
    /// The encrypted archive's password field.
    ArchivePassword(String),
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
    /// A side was scrolled: (side, x, y); the other one follows.
    DiffScrolled(usize, f32, f32),
    /// The × on a message card: the window's message (`None`) or a pane's read error.
    Dismiss(Option<usize>),
    DiffOpt(dialogs::DiffOpt),
    /// Read both files again (TC "Compare" / rescan); not with unsaved changes.
    DiffReread,
    /// Copy the current block of differences left → right (true) or right → left.
    DiffCopy(bool),
    DiffUndo,
    /// «Edit left / right» (0 / 1): that side's text in an editor.
    DiffEdit(usize),
    DiffEditAct(cosmic::iced::widget::text_editor::Action),
    /// Done (`true`: compare the edited text again) or Cancel.
    DiffEditDone(bool),
    DiffSave,
    /// Alt+F7: the "regular expression" checkbox.
    FindRegex,
    /// «Искать текст» on / off.
    FindTextOn,
    /// «? Регулярные выражения»: the hints shown / hidden.
    FindHelp,
    FindNameRegex,
    FindArchives,
    /// Alt+Enter: toggle a permission bit / "also inside folders".
    PropsBit(u32),
    /// Alt+Enter's group drop-down: index into `Props::gids`.
    PropsGroup(usize),
    /// Its owner drop-down: index into `Props::uids`.
    PropsUser(usize),
    PropsRecursive,
    /// id, usage counted in the background (`None` = stopped).
    PropsUsage(u64, Option<shagoff_core::props::Usage>),
    /// Space on a dir counted it: (tab id, its parent, its name, bytes).
    DirSize(u64, PathBuf, OsString, u64),
    /// chmod finished: (side, what failed).
    PropsDone(usize, Vec<String>),
    /// Alt+F7: the results into the dialog's panel (TC "Feed to listbox").
    FindFeed,
    /// Alt+L, by the physical key (any layout): "To panel" in the search window, otherwise the
    /// action it would have been (quick search; `None` from a text field).
    FeedKey(Option<Action>),
    Op(jobs::Event),
    Resolve(Resolution),
    ErrorAnswer(ErrorChoice),
    CancelJob,
    /// The progress dialog's "In background" / the status line's "Show".
    JobHide,
    JobShow,
    /// Pause / go on with the running job.
    JobPause,
    /// ✕ on a queue row: it will not run.
    Unqueue(usize),
    /// ↑ on a queue row: it runs one place earlier.
    QueueUp(usize),
    /// A panel key a focused text field captured (F-keys, PgUp/PgDn, Insert, Ctrl+…).
    FieldKey(Action),
    /// Click on entry i of the open list dialog.
    ListPick(usize),
    /// Alt+↓ / Alt+F8 "Clear": forget that history.
    ListClear,
    /// "Open with…": the files (taken when asked) and the programs found for the first one.
    OpenWithApps(usize, Vec<PathBuf>, Result<openwith::Found, String>),
    /// Ctrl+B read: shown if that tab is still in `root`, as it was.
    BranchListed {
        tab: u64,
        /// The request: dropped unless the tab still waits for this one.
        id: u64,
        root: PathBuf,
        /// `None`: stopped.
        paths: Option<Vec<PathBuf>>,
    },
    /// "Open with…" → "All programs…": every installed one.
    OpenWithAll(Vec<openwith::App>),
    /// "Open with…" → "Default": the program under the list's cursor for this type.
    OpenWithDefault,
    /// Its result: the program's name, or the error.
    OpenWithDefaultSet(Result<String, String>),
    /// Hotlist settings (Ctrl+D → "Configure…").
    Hot(HotMsg),
    /// Configuration → "Colors by type…".
    Colors(crate::colorrules::ColorMsg),
    /// Toolbar button `i` of `config.toolbar`.
    Tool(usize),
    /// Shift + drag on the toolbar (TC): the pointer over / off a button, the left button down /
    /// up (from a subscription: the buttons keep the press).
    ToolHover(Option<usize>),
    ToolLeft(usize),
    ToolGrab,
    ToolRelease,
    /// A toolbar program exited with an error: its last stderr line (or exit status).
    ToolFailed(String),
    ToolEdit(crate::toolbar::ToolMsg),
    KeyEdit(crate::hotkeys::KeyMsg),
    /// Settings → «Find a font» typed.
    FontFilter(String),
    /// Text typed into the quick search / filter field.
    SearchInput(String),
    /// Files read from the system clipboard by Ctrl+V (`None`: no files there).
    Pasted(Option<(ClipKind, Vec<PathBuf>)>),
    /// Files dropped on a pane (`dir`: the dir row they were dropped on); `mv`: Shift was held.
    Drop {
        side: usize,
        dir: Option<PathBuf>,
        paths: Vec<PathBuf>,
        mv: bool,
    },
    /// A row drag started; `Some(files)`: with the right button (their drop asks what to do).
    DragStart(Option<Arc<Vec<PathBuf>>>),
    /// Our row drag ended (dropped or cancelled).
    DragEnd,
    /// Right-button drop menu: Copy / Move / Link.
    DropAs(InputOp),
    /// The pointer is over drop menu row `i`: the menu cursor follows it, as in TC.
    DropRow(usize),
    /// Where held files are now (window coordinates): a right-button drop's menu opens there.
    DropAt(f64, f64),
    /// Files are held over a dir row (it lights up) / no longer over any.
    DropHover(Option<(usize, usize)>),
    /// The pointer left that row while dragging.
    DropLeave((usize, usize)),
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
    /// Full view header: press on edge 1..=4 (TC drag), pointer x, release.
    /// The divider between the panes: pressed, the pointer at window x while held, let go,
    /// double-clicked (half and half).
    SplitStart,
    SplitAt(cosmic::iced::window::Id, f32),
    /// The main window's width, for the divider (`Opened` / `Resized`).
    WindowWidth(cosmic::iced::window::Id, f32),
    SplitEnd,
    SplitReset,
    ColDragStart(usize, usize),
    ColDrag(f32),
    ColDragEnd,
    /// A part of the path line clicked: go there, the cursor on the dir we came from.
    PathPart(usize, PathBuf),
    CloseDrawer,
    Setting(Setting),
    /// Tab in a settings field.
    FocusNext,
    /// A link in the about drawer.
    OpenUrl(String),
    /// About → «Check for updates», and its answer (the latest tag).
    CheckUpdate,
    UpdateChecked(shagoff_core::update::Status),
    /// Ctrl+F dialog: the password field.
    ConnectPassword(String),
    /// «Remember the password» in Ctrl+F.
    ConnectKeep(bool),
    /// The password went to the keyring, or why not.
    Kept(Result<(), mount::Error>),
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
    Config(Box<Config>),
    /// Ctrl+F finished: (side, address, mount result).
    Connected(usize, String, Result<PathBuf, mount::Error>),
    /// Ctrl+F: a saved or found address into the field.
    ConnectPick(String),
    /// Ctrl+F: forget this saved address.
    ConnectForget(String),
    /// Browse a network place (`None`: `network:///` again; pressed while looking: stop).
    ConnectBrowse(Option<String>),
    /// What "Browse network" found.
    /// What was found by the browse with this stop flag (a stopped or replaced one is dropped).
    Browsed(Arc<AtomicBool>, Result<Vec<(String, String)>, mount::Error>),
    /// File read for the viewer with this id.
    ListerLoaded(u64, Arc<Result<lister::Loaded, String>>),
    /// Another window of a big file for the viewer with this id (`Lister::slide`).
    ListerPart(u64, Slide, Arc<Result<lister::Loaded, String>>),
    /// The viewer's picture is in the renderer (redraws with it).
    ListerAllocated(u64, Option<cosmic::iced::runtime::image::Allocation>),
    /// Nothing but a redraw: see `subscription`.
    ListerWake,
    ListerKey(ListerKey),
    /// A viewer letter key typed in the panel: the viewer's when it is open, else the command line's.
    Letter(ListerKey, char),
    /// x, y scroll offset, viewport height.
    ListerScrolled(f32, f32, f32),
    /// Viewer selection: pointer over the text (viewport px), left button down / up.
    ListerPointer(f32, f32),
    ListerPress,
    ListerRelease,
    ListerPointerLeft,
    /// Comes instead of the second release.
    ListerDoubleClick,
    /// The picture back to fitted in the window.
    ListerFit,
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
    /// The viewer's own window: its title bar.
    ListerWindow(ListerWin),
    /// A window is gone (the viewer's: the viewer closes).
    WindowClosed(cosmic::iced::window::Id),
    /// The built-in terminal's pending `cd`: ask the shell again (`term::waiting_tick`).
    TermWait,
    /// Every 2 s while files from archives are in the editor: look for saved changes.
    EditTick,
    /// Window ×, Alt+F4, menu «Exit»: asks first while a job runs or waits.
    Quit,
    /// Alt+F4 / the compositor's close on window `Id` (`exit_on_close(false)` keeps it open).
    CloseRequested(cosmic::iced::window::Id),
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
        // Only to have the windows redrawn (`subscription`): no state changes to follow.
        if let Message::ListerWake = message {
            return Task::none();
        }
        let shown = self.viewer_shown();
        let task = self.handle(message);
        if self.dialog.is_none() {
            self.dialog = self.held.take();
        }
        let follow = self.quick_follow();
        // Ctrl+Shift+S's options as on screen, however the dialog then closes.
        if let Some(Dialog::Sync(s)) = &self.dialog {
            self.sync = config::SyncPrefs {
                recursive: s.recursive,
                content: s.content,
                ignore_date: s.ignore_date,
                mask: s.mask.clone(),
                hide: s
                    .hide
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            };
        }
        self.save_state();
        // The viewer came, went or moved to the other pane: the lists it replaced are new
        // widgets at the top, while the tabs keep their offsets (the rows drawn are those).
        let back = match self.viewer_shown() == shown {
            true => Task::none(),
            false => Task::batch([self.restore_scroll(0), self.restore_scroll(1)]),
        };
        let back = Task::batch([back, self.roam_back()]);
        self.term_follow();
        let waiting = matches!(&self.dialog, Some(Dialog::Keys(k)) if k.capturing);
        keymap::CAPTURE.store(waiting, Ordering::Relaxed);
        let blur = if std::mem::take(&mut self.term_blur) {
            unfocus()
        } else if self.term_want_focus && self.term.is_some() {
            // Turned on by Ctrl+`: its keys once its shell is up.
            self.term_want_focus = false;
            cosmic::iced::runtime::widget::operation::focus(self.term_id.clone())
        } else {
            Task::none()
        };
        Task::batch([
            task,
            follow,
            back,
            self.want_thumbs(),
            self.read_gvfs(),
            blur,
        ])
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            event::listen_with(route_event),
            // Only while a tab is held: where the pointer is (the panes' own widgets take the
            // moves, so a pane's mouse area would not hear it come in).
            match self.tab_drag {
                Some(_) => event::listen_with(|e, _, window| match e {
                    cosmic::iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        Some(Message::TabDragAt(window, position.x))
                    }
                    _ => None,
                }),
                None => Subscription::none(),
            },
            // «Настроить…» of Ctrl+D: the left button anywhere (its rows are buttons, which keep
            // the press) holds / lets go of the row under the pointer.
            match self.dialog {
                Some(Dialog::Hotlist(_)) => event::listen_with(|e, _, _| match e {
                    cosmic::iced::Event::Mouse(mouse::Event::ButtonPressed(
                        mouse::Button::Left,
                    )) => Some(Message::Hot(HotMsg::Grab)),
                    cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(
                        mouse::Button::Left,
                    )) => Some(Message::Hot(HotMsg::Release)),
                    // Scrolled: another row is under the pointer than the last one entered.
                    cosmic::iced::Event::Mouse(mouse::Event::WheelScrolled { .. }) => {
                        Some(Message::Hot(HotMsg::Hover(None)))
                    }
                    _ => None,
                }),
                _ => Subscription::none(),
            },
            // Shift held or a toolbar button held: the left button down / up anywhere.
            match self.mods.shift() || self.tool_drag.grab.is_some() {
                true => event::listen_with(|e, _, _| match e {
                    cosmic::iced::Event::Mouse(mouse::Event::ButtonPressed(
                        mouse::Button::Left,
                    )) => Some(Message::ToolGrab),
                    cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(
                        mouse::Button::Left,
                    )) => Some(Message::ToolRelease),
                    _ => None,
                }),
                false => Subscription::none(),
            },
            // While the divider is held: where the pointer is, and the release (over any widget).
            match self.splitting {
                true => event::listen_with(|e, _, window| match e {
                    cosmic::iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        Some(Message::SplitAt(window, position.x))
                    }
                    // Let go, or gone out of the window (the release may never reach us).
                    cosmic::iced::Event::Mouse(
                        mouse::Event::ButtonReleased(mouse::Button::Left)
                        | mouse::Event::CursorLeft,
                    ) => Some(Message::SplitEnd),
                    _ => None,
                }),
                false => Subscription::none(),
            },
            // Alt+F4 / the compositor's close: the window stays until Quit decides.
            cosmic::iced::window::close_requests().map(Message::CloseRequested),
            self.core()
                .watch_config::<Config>(APP_ID)
                .map(|u| Message::Config(Box::new(u.config))),
        ];
        // The viewer's picture handed to the renderers: their "redraw, it's uploaded" is lost while
        // the main window isn't drawn (covered by the viewer's window), so a tick wakes the loop a
        // while (regression: F3 showed nothing until a key). Each tick recomputes this: it stops.
        if (self.lister.as_ref().and_then(|l| l.wake_until))
            .is_some_and(|t| std::time::Instant::now() < t)
        {
            subs.push(
                cosmic::iced::time::every(std::time::Duration::from_millis(100))
                    .map(|_| Message::ListerWake),
            );
        }
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
        if self.config.term_builtin {
            subs.push(crate::term::subscription());
        }
        // A `cd` waiting for the shell's prompt: `term_follow` (after every update) asks again.
        if let Some(tick) = self
            .term
            .as_ref()
            .and_then(|t| crate::term::waiting_tick(t))
        {
            subs.push(tick.map(|()| Message::TermWait));
        }
        // The terminal's top edge held: the pointer anywhere, and where the button comes up.
        if self.term_drag.is_some() {
            subs.push(event::listen_with(|e, _, _| match e {
                cosmic::iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::TermDragAt(position.y))
                }
                cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::TermDragEnd)
                }
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }

    /// The header bar's ×: asks first while a job runs or waits (`exit_on_close(false)`).
    fn on_app_exit(&mut self) -> Option<Message> {
        Some(Message::Quit)
    }

    /// The window is gone (closed by something other than Quit): the same temp cleanup as Exit.
    /// Only for the main window: a context menu's popup closes through here too.
    fn on_close_requested(&self, id: cosmic::iced::window::Id) -> Option<Message> {
        if self.lister.as_ref().is_some_and(|l| l.window == Some(id)) {
            return Some(Message::ListerClose);
        }
        (id == self.window_id()).then_some(Message::Exit)
    }

    fn view_window(&self, id: cosmic::iced::window::Id) -> Element<'_, Message> {
        crate::view::lister_window(self, id)
    }

    fn context_drawer(&self) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        self.drawer.as_ref().map(|d| drawer::view(self, d))
    }

    fn header_start(&self) -> Vec<Element<'_, Message>> {
        let c = &self.config;
        let checks = crate::menu::Checks {
            hidden: c.show_hidden,
            divider_line: c.divider_line,
            human_sizes: c.human_sizes(),
            active_frame: c.active_frame,
            classic: c.skin == config::Skin::Classic,
            terminal: c.term_builtin,
            show: match self.panes[self.active].active().panel.show() {
                Show::All => 0,
                Show::Programs => 1,
                Show::Mask(_) => 2,
            },
        };
        vec![crate::menu::bar(
            checks,
            self.panes[self.active].active().lock(),
        )]
    }

    fn view(&self) -> Element<'_, Message> {
        crate::view::view(self)
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        if let Some(d) = &self.dialog {
            // A list no taller than the panel's rows: the dialog's buttons stay in the window.
            let list_h = self.panes[self.active].active().height;
            let font = self.panel_font();
            // The single file's icon from the panel row it came from (no disk access here).
            let icon = match d.sources() {
                Some([one]) => self.panes.iter().find_map(|p| {
                    let panel = &p.active().panel;
                    panel
                        .entries()
                        .iter()
                        .find(|e| panel.cwd().join(&e.os_name) == *one)
                }),
                _ => None,
            }
            .map(|e| crate::view::entry_icon(e, 32));
            return Some(dialogs::view(
                d,
                &self.input_id,
                &self.tz,
                list_h,
                font,
                icon,
            ));
        }
        self.job
            .as_ref()
            .filter(|j| !j.hidden)
            .map(|j| dialogs::progress(j, &self.queue))
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
        let cols = config.columns;
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
            update_check: None,
            job: None,
            queue: VecDeque::new(),
            held: None,
            scheme: shagoff_core::colors::Scheme::new(&config.colors).with_panel(&config.panel),
            face: font_named(&config.panel_font),
            right_drag: None,
            right_hold: None,
            right_presses: 0,
            dragging: false,
            quitting: false,
            input_id: widget::Id::unique(),
            search: None,
            home,
            config,
            config_handler: None,
            state_handler: None,
            saved: State::default(),
            find: state.find.clone(),
            sync: state.sync.clone(),
            split: state.split,
            splitting: false,
            split_from: None,
            win_w: 0.0,
            drives: Vec::new(),
            terminals: Vec::new(),
            term_default: None,
            term_tx: None,
            term: None,
            term_full: false,
            term_id: widget::Id::unique(),
            term_ended: false,
            term_drag: None,
            term_rows: 0,
            term_blur: false,
            testing: false,
            term_focused: false,
            term_want_focus: false,
            gvfs: Vec::new(),
            gvfs_want: false,
            gvfs_busy: false,
            cols,
            col_drag: None,
            tab_drag: None,
            pane_hover: None,
            drop_hover: None,
            tool_drag: Default::default(),
            thumbs: HashMap::new(),
            thumbs_busy: 0,
            drop_at: None,
            hot_sub: Vec::new(),
            volumes: Vec::new(),
            volumes_at: None,
            space: [None, None],
            status: None,
            open_with: Vec::new(),
            open_with_mime: String::new(),
            quick_latest: Arc::default(),
            quick_stop: Arc::default(),
            owners: shagoff_core::owners::Owners::load(),
            ctx_entry: false,
            ctx_row: false,
            lister: None,
            edited: Vec::new(),
            nested: HashMap::new(),
            shown_tabs: [0; 2],
            updating: None,
            connecting: None,
            cmdline: String::new(),
            cmd_id: widget::Id::unique(),
            commands: state.commands.clone(),
            masks: state.masks.clone(),
            other_cmds: state.other_cmds.clone(),
        };
        // A file opens its folder; a missing path keeps the saved tab.
        let left = left
            .and_then(|p| p.canonicalize().ok())
            .map(|p| session::existing_dir(&p, &home_fallback));
        for side in 0..2 {
            let (mut paths, active) = session::restore(&state.panes[side], &app.home);
            let saved = &state.panes[side];
            let locked = |i: usize| saved.locked.get(i).copied().unwrap_or(false);
            let roam = |i: usize| saved.roam.get(i).copied().unwrap_or(false);
            // A locked tab keeps its dir: the path opens in a new tab next to it.
            let extra = match &left {
                Some(l) if side == 0 && (locked(active) || roam(active)) => Some(l.clone()),
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
                if roam(i) {
                    t.home = Some(t.panel.cwd().to_path_buf());
                }
                t.thumbs = saved.thumbs.get(i).copied().unwrap_or(false);
                t.brief = !t.thumbs && saved.brief.get(i).copied().unwrap_or(false);
                t.name = saved.names.get(i).filter(|n| !n.is_empty()).cloned();
                t.history = saved.history.get(i).cloned().unwrap_or_default();
            }
            if let Some(l) = extra {
                tabs.open_after(app.new_tab(l));
            }
            app.panes[side] = tabs;
        }
        keymap::set_own(crate::hotkeys::resolve(&app.config.hotkeys));
        app.refresh_mounts();
        let task = Task::batch([app.load_all(), app.read_gvfs(), scan_terminals()]);
        (app, task)
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            // The focused terminal took these (F8 in mc, Ctrl+R in bash, Esc in vim): a key a
            // text field leaves to us, `route_event` passes on at any status — not this one's.
            Message::FieldKey(_) | Message::Escape(_) | Message::FeedKey(_)
                if self.term_focused && self.dialog.is_none() => {}
            Message::TermFocused(on) => self.term_focused = on,
            Message::Key(action) if self.viewer() && self.dialog.is_none() && !self.busy() => {
                if let Some(task) = self.lister_action(action) {
                    return task;
                }
                if action == Action::RunTerminal {
                    return Task::none(); // not the panel's file from under the viewer
                }
                return self.act(self.active, action);
            }
            Message::Key(action) => {
                // The drop menu as TC's: ↑ / ↓ over Copy, Move, Link, Cancel; Enter picks.
                if let Some(Dialog::DropMenu {
                    cursor, sources, ..
                }) = &mut self.dialog
                {
                    let ops = dialogs::drop_ops(sources);
                    let last = ops.len();
                    match action {
                        Action::Up => *cursor = cursor.checked_sub(1).unwrap_or(last),
                        Action::Down => *cursor = (*cursor + 1) % (last + 1),
                        Action::Enter => {
                            let op = ops.get(*cursor).copied();
                            return self.handle(op.map_or(Message::DialogCancel, Message::DropAs));
                        }
                        _ => {}
                    }
                    return Task::none();
                }
                if let Some(Dialog::List {
                    kind,
                    cursor,
                    items,
                    ..
                }) = &self.dialog
                    && *kind == ListKind::Hotlist
                {
                    let row = items.get(*cursor).map(|it| it.kind);
                    match (action, row) {
                        (Action::Delete | Action::DeletePermanent, Some(Item::Hot(i))) => {
                            return self.hotlist_remove(i);
                        }
                        (Action::Right, Some(Item::Sub(_))) => {
                            let i = *cursor;
                            return self.pick(i);
                        }
                        (Action::Left | Action::Parent, _) if !self.hot_sub.is_empty() => {
                            self.hotlist_back();
                            return Task::none();
                        }
                        _ => {}
                    }
                }
                if let Some(Dialog::Tree {
                    tree, hidden, id, ..
                }) = &mut self.dialog
                {
                    const PAGE: isize = 10;
                    match action {
                        Action::Up => tree.move_cursor(-1),
                        Action::Down => tree.move_cursor(1),
                        Action::PageUp => tree.move_cursor(-PAGE),
                        Action::PageDown => tree.move_cursor(PAGE),
                        Action::Home => tree.move_cursor(-isize::MAX / 2),
                        Action::End => tree.move_cursor(isize::MAX / 2),
                        // → opens it (read in the background), then steps in; ← back out.
                        Action::Right => {
                            if let Some(dir) = tree.step_in() {
                                return tree_read(*id, vec![dir], *hidden, None);
                            }
                        }
                        Action::Left | Action::Parent => tree.step_out(),
                        Action::Enter => return self.submit_dialog(),
                        _ => return Task::none(),
                    }
                    return list_snap(tree.cursor, tree.rows.len());
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
                        _ => return Task::none(),
                    }
                    return list_snap(*cursor, items.len());
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
                        && matches!(
                            d,
                            Dialog::ConfirmDelete { .. }
                                | Dialog::Props(_)
                                | Dialog::Checksum { .. }
                        )
                    {
                        return self.submit_dialog();
                    }
                    // Through its buttons and fields, as in TC: arrows and Tab (a field keeps ← →
                    // for itself); Enter presses the focused button (the button takes it).
                    use cosmic::iced::widget::operation::{focus_next, focus_previous};
                    return match action {
                        Action::Down | Action::Right | Action::SwitchPane => focus_next(),
                        Action::Up | Action::Left => focus_previous(),
                        _ => Task::none(),
                    };
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
                                (p.cursor() + n).saturating_sub(1) // empty list: none found
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
                        // The field lost focus (a click): Alt+letter goes on with its text.
                        Action::QuickSearch(_) if !filter => {}
                        _ => self.search = None, // any other key closes the field and acts
                    }
                }
                // Only the Enter key runs a typed line (the field lost focus to a click); a double
                // click or quick search's Enter opens the entry.
                if matches!(action, Action::Enter | Action::RunTerminal)
                    && self.cmd_ready()
                    && !self.cmdline.trim().is_empty()
                {
                    return self.cmd_run(action == Action::RunTerminal);
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
                let Some(side) = self.side_of(tab) else {
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
                // Ctrl+B's rows are in: the branch is read.
                if t.branching
                    .take_if(|_| t.branch && t.results.is_some())
                    .is_some()
                    && (self.status.as_ref()).is_some_and(|s| s.kind == StatusKind::Busy)
                {
                    self.status = None;
                }
                let Some(t) = (self.panes[side].items_mut().iter_mut()).find(|t| t.id == tab)
                else {
                    return Task::none();
                };
                match result {
                    Ok(entries) => {
                        t.error = None;
                        t.history.visit(&path); // no-op for a rescan of the current entry
                        let moved = path != t.panel.cwd();
                        if moved {
                            t.counting.store(true, Ordering::Relaxed);
                            t.counting = Arc::default();
                        }
                        t.panel.set_listing(path, entries, focus.as_deref());
                        let shown = self.panes[side].active().id == tab;
                        if shown {
                            self.refresh_mounts();
                            self.space[side] = space;
                        }
                        // The tree's tab went elsewhere (Alt+F10, Ctrl+D…): the tree opened there.
                        let tree = self.panes[side].active().tree.is_some();
                        if shown && moved && tree {
                            return Task::batch([self.reveal(side, tab), self.tree_open(side)]);
                        }
                        return self.reveal(side, tab);
                    }
                    Err(e) => {
                        // An Alt+← / Alt+→ step that failed: history back on the dir shown.
                        let shown = t.panel.cwd().to_path_buf();
                        t.history.seat(&shown);
                        let ask = archive::needs_password(&e)
                            .then(|| archive::split_path(&path))
                            .flatten();
                        t.error = Some(fl!(
                            "list-failed",
                            path = path.display().to_string(),
                            err = crate::why::message(&e)
                        ));
                        // A 7z with encrypted names: ask, then open it again.
                        if let Some((arc, _)) = ask
                            && self.dialog.is_none()
                        {
                            self.dialog = Some(Dialog::Password {
                                wrong: archive::password(&arc).is_some(),
                                archive: arc,
                                password: String::new(),
                                reply: None,
                                open: Some((side, path)),
                            });
                            return widget::text_input::focus(self.input_id.clone());
                        }
                    }
                }
            }
            Message::Click(side, i) => {
                self.click_pane(side);
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                if self.mods.control() {
                    t.panel.toggle_mark();
                }
                let tab = t.id;
                return self.reveal(side, tab); // a half-visible row scrolls fully in
            }
            Message::RightClick(side, i) if self.config.right_marks => {
                self.click_pane(side);
                self.ctx_row = true;
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                let tab = t.id;
                return Task::batch([self.reveal(side, tab), self.right_press(side, Some(i))]);
            }
            Message::RightEmpty(side) if self.config.right_marks => {
                self.click_pane(side);
                // The same press on a row came first: that row's hold stands.
                if !std::mem::take(&mut self.ctx_row) {
                    return self.right_press(side, None);
                }
            }
            Message::RowEnter(side, i) => {
                let Some(h) = &mut self.right_hold else {
                    return Task::none();
                };
                let t = self.panes[side].active_mut();
                let Some(start) = h.row.filter(|_| h.side == side && h.tab == t.id) else {
                    return Task::none();
                };
                let (on, last) = match h.drag {
                    Some(d) => d,
                    None if i == start => return Task::none(),
                    // Off the pressed row: a drag, marking as that row would have become.
                    None => {
                        let e = t.panel.entries().get(start);
                        (!e.is_some_and(|e| t.panel.is_marked(e)), start)
                    }
                };
                h.drag = Some((on, i));
                // From the last row reached: a quick move skips rows' enter events.
                t.panel.mark_range(last, i, on);
                t.panel.set_cursor(i);
                let tab = t.id;
                return self.reveal(side, tab);
            }
            Message::RightHeld(n) => {
                if let Some(h) = &mut self.right_hold
                    && h.n == n
                    && h.drag.is_none()
                {
                    h.armed = true;
                    self.ctx_entry = h.row.is_some();
                }
            }
            Message::RightUp => {
                let Some(h) = self.right_hold.take() else {
                    return Task::none();
                };
                if h.drag.is_some() {
                    return Task::none(); // marked while dragging: done
                }
                let t = self.panes[h.side].active_mut();
                // Only where it was pressed: not after a tab switch, or with the cursor moved.
                if t.id != h.tab || h.row.is_none_or(|r| r != t.panel.cursor()) {
                    return Task::none();
                }
                if !h.armed {
                    t.panel.toggle_mark(); // a quick click: TC marks the row
                } else if !t.panel.current().is_some_and(|e| t.panel.is_marked(e)) {
                    // The menu opens now, as a right click's at once: about the unmarked row only.
                    t.panel.mark_all(false);
                }
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
                let shrunk = size.height < t.height || t.brief || t.thumbs;
                for t in self.panes[side].items_mut() {
                    (t.width, t.height) = (size.width, size.height);
                }
                if shrunk {
                    let tab = self.panes[side].active().id;
                    return self.reveal(side, tab); // keep the cursor on screen
                }
            }
            Message::Thumb(path, mtime, pic) => {
                self.thumbs_busy -= 1;
                // Changed since it was asked: the next `want_thumbs` asks again.
                if let Some(slot) = self.thumbs.get_mut(&path)
                    && slot.0 == mtime
                {
                    slot.1 = pic;
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
                let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
                let last = t.panel.entries().len().div_ceil(rows).saturating_sub(cols);
                t.col = if y > 0.0 {
                    t.col.saturating_sub(1)
                } else if y < 0.0 {
                    (t.col + 1).min(last)
                } else {
                    t.col
                };
            }
            Message::TabPress(side, i) => {
                self.tab_drag = Some((side, i));
                return self.update(Message::SelectTab(side, i));
            }
            Message::TabEnter(side, i) => {
                if let Some((s, from)) = &mut self.tab_drag
                    && *s == side
                    && *from != i
                {
                    self.panes[side].move_to(*from, i);
                    *from = i;
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
                if self.panes[side].items().get(i).is_some_and(Tab::pinned) {
                    self.say(StatusKind::Error, fl!("tab-locked"));
                    return Task::none();
                }
                self.panes[side].close(i);
                return self.tab_switched(side);
            }
            Message::Modifiers(m) => {
                self.mods = m;
                // Shift let go (or the window left, which clears them): a held button is dropped
                // where it is — the release may never come.
                if !m.shift() {
                    self.tool_drop();
                }
            }
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
            Message::MrInsert(tok) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.name.push_str(tok);
                    return Task::batch([
                        widget::text_input::focus(self.input_id.clone()),
                        widget::text_input::move_cursor_to_end(self.input_id.clone()),
                    ]);
                }
            }
            Message::MrHelp => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.help = !m.help;
                }
            }
            Message::MrCase(c) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.case = c;
                }
            }
            Message::SumAlgo(a) => {
                if let Some(Dialog::Checksum { algo, .. }) = &mut self.dialog {
                    *algo = a;
                }
                let v = a.ext().to_string();
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_checksum_algo(h, v) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.checksum_algo = v,
                }
            }
            Message::PackFormat(f) => {
                if let Some(Dialog::Pack(p)) = &mut self.dialog {
                    p.format = f;
                    p.path = archive::with_format(&p.path, f);
                }
                self.save_pack_format(f);
            }
            Message::ArchivePassword(s) => {
                if let Some(Dialog::Password { password, .. }) = &mut self.dialog {
                    *password = s;
                }
            }
            Message::PackPassword(again, s) => {
                if let Some(Dialog::Pack(p)) = &mut self.dialog {
                    *if again { &mut p.again } else { &mut p.password } = s;
                }
            }
            Message::Toggle(t) => match (&mut self.dialog, t) {
                (Some(Dialog::Pack(p)), Toggle::MoveAfter) => p.move_after = !p.move_after,
                (Some(Dialog::Pack(p)), Toggle::Separate) => p.separate = !p.separate,
                (Some(Dialog::Pack(p)), Toggle::Encrypt) => {
                    p.encrypt = !p.encrypt;
                    if p.encrypt {
                        return widget::text_input::focus(widget::Id::new("pack-password"));
                    }
                }
                (Some(Dialog::Unpack { own_dir, .. }), Toggle::OwnDir) => *own_dir = !*own_dir,
                (Some(Dialog::Checksum { separate, .. }), Toggle::SumSeparate) => {
                    *separate = !*separate
                }
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
                        FindField::Attrs => &mut f.attrs,
                    } = s;
                }
            }
            Message::FindCase => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.case_sensitive = !f.case_sensitive;
                    f.in_list = false;
                }
            }
            Message::FindTextOn => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.find_text = !f.find_text;
                    f.in_list = false;
                }
            }
            Message::FindHelp => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.help = !f.help;
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
            Message::PropsGroup(i) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && i < p.gids.len()
                {
                    p.group = Some(i);
                }
            }
            Message::PropsUser(i) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && i < p.uids.len()
                {
                    p.user = Some(i);
                }
            }
            Message::PropsBit(b) => {
                // In an archive the bits are only shown.
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && p.archive.is_none()
                {
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
                    p.mixed = if p.recursive { 0o7777 } else { p.own_mixed };
                }
            }
            Message::DirSize(tab, cwd, name, bytes) => {
                // By id: the tab may have moved to the other panel meanwhile (Ctrl+U, a drag).
                if let Some(side) = self.side_of(tab)
                    && let Some(t) = self.panes[side]
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
            Message::FeedKey(other) => {
                // As the button: nothing to feed, nothing done.
                if let Some(Dialog::Find(f)) = &self.dialog {
                    return match f.results.is_empty() {
                        true => Task::none(),
                        false => self.find_feed(),
                    };
                }
                if let Some(a) = other {
                    return self.update(Message::Key(a));
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
                            d.undo.push((before, d.dirty));
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
            Message::DiffScrolled(side, x, y) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    // The follower reports the same offset back: stop there.
                    if (x - d.x).abs() < 0.5 && (y - d.offset).abs() < 0.5 {
                        return Task::none();
                    }
                    (d.x, d.offset) = (x, y);
                    // Scrolled away from the current block: the first one in view takes over.
                    let (top, bottom) = d.view_rows();
                    if let Some(t) = d.text() {
                        d.block = shagoff_core::diff::block_in_view(
                            &t.rows,
                            d.blocks(),
                            d.block,
                            top,
                            bottom,
                        );
                    }
                    let to = AbsoluteOffset {
                        x: Some(x),
                        y: Some(y),
                    };
                    return scrollable::scroll_to(d.scroll[1 - side].clone(), to);
                }
            }
            Message::DiffReread => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.result.is_some()
                    && !(d.dirty.0 || d.dirty.1)
                {
                    d.block = 0;
                    return self.start_diff();
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
                    return self.start_diff_with(Some(shown), None, None);
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
                    return self.start_diff_with(Some(shown), Some((block, to_right)), None);
                }
            }
            Message::DiffEdit(side) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                {
                    let text = if side == 0 { &src.0 } else { &src.1 };
                    let mut content = text_editor::Content::with_text(text);
                    // On the current block's line, where the change is.
                    let row = d.blocks().get(d.block).copied().unwrap_or(0);
                    let line = d
                        .text()
                        .map_or(0, |t| shagoff_core::diff::line_at(&t.rows, row, side));
                    // ponytail: the view follows the cursor when drawn, so the line shows at the
                    // bottom edge; centring it would need the editor's own scroll.
                    for _ in 0..line {
                        content.perform(text_editor::Action::Move(text_editor::Motion::Down));
                    }
                    d.editing = Some((side, content));
                    d.confirm_close = false;
                    return cosmic::iced::widget::operation::focus(d.edit_id.clone());
                }
            }
            Message::DiffEditAct(a) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some((_, content)) = &mut d.editing
                {
                    content.perform(a);
                }
            }
            Message::DiffEditDone(apply) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some((side, content)) = d.editing.take()
                    && apply
                    && let Some(src) = d.text().map(|t| t.src.clone())
                    && let Some(texts) = shagoff_core::diff::edited(&src, side, &content.text())
                {
                    // As a copied block: undoable, that side unsaved (`DiffReady`).
                    d.copying = Some((side == 1, src));
                    return self.start_diff_with(None, None, Some(Arc::new(texts)));
                }
            }
            Message::DiffUndo => {
                // Not while a compare runs: it may hold a copy that would land after the undo.
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.text().is_some()
                    && let Some((src, dirty)) = d.undo.pop()
                {
                    d.dirty = dirty;
                    d.confirm_close = false;
                    return self.start_diff_with(None, None, Some(src));
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
                    match failed {
                        // A side still unsaved: its undo history and the close warning stay.
                        Some(e) => self.say(StatusKind::Error, e),
                        None => {
                            d.confirm_close = false;
                            // The disk now holds what undoing would have changed.
                            d.undo.clear();
                            self.say(StatusKind::Info, fl!("diff-saved"));
                        }
                    }
                }
            }
            Message::DiffNext => return self.diff_step(1),
            Message::DiffPrev => return self.diff_step(-1),
            Message::DialogSubmit => return self.submit_dialog(),
            Message::DialogQueue => {
                let idle = self.job.is_none();
                let task = self.submit_dialog();
                if idle && let Some(j) = &mut self.job {
                    j.hidden = true;
                }
                return task;
            }
            Message::ListerLoaded(id, loaded) => {
                if let Some(l) = &mut self.lister
                    && l.id == id
                {
                    l.set_loaded(loaded);
                    if let Some(h) = l.raster_handle() {
                        l.wake_until = Some(std::time::Instant::now() + lister::WAKE_MAX);
                        return cosmic::iced::runtime::image::allocate(h).map(move |a| {
                            cosmic::Action::App(Message::ListerAllocated(id, a.ok()))
                        });
                    }
                }
            }
            Message::ListerPart(id, slide, loaded) => {
                let Some(l) = self.lister.as_deref_mut().filter(|l| l.id == id) else {
                    return Task::none();
                };
                if let Err(e) = loaded.as_ref() {
                    if l.slide == Some(slide) {
                        l.slide = None;
                        let e = e.clone();
                        self.say(StatusKind::Error, e);
                    }
                    return Task::none();
                }
                let part = l.set_window(slide, loaded);
                let q = l.query.clone();
                let scroll = part
                    .scroll
                    .map_or(Task::none(), |y| lister_scroll(l, l.offset.0, y));
                match part.found {
                    Some(true) => self.clear_busy(),
                    Some(false) => self.say(StatusKind::Info, fl!("lister-not-found", query = q)),
                    None => {}
                }
                let read = part.read.map_or(Task::none(), |s| self.lister_slide(s));
                return Task::batch([scroll, read]);
            }
            Message::ListerWake => {}
            Message::ListerAllocated(id, a) => {
                if let Some(l) = &mut self.lister
                    && l.id == id
                {
                    // Held in the main window's renderer; the viewer's own may need a frame or
                    // two more. Failed: that one loads it alone, the whole while.
                    if a.is_some() {
                        let soon = std::time::Instant::now() + lister::WAKE_AFTER;
                        l.wake_until = l.wake_until.map(|t| t.min(soon));
                    }
                    l.allocation = a;
                }
            }
            Message::ListerKey(k) if self.viewer() && self.dialog.is_none() && !self.busy() => {
                return match k {
                    ListerKey::Mode(m) => self.handle(Message::ListerMode(m)),
                    ListerKey::Next => self.lister_step(true),
                    ListerKey::Prev => self.lister_step(false),
                    ListerKey::Close => self.handle(Message::ListerClose),
                    ListerKey::Zoom(up) => self.lister_zoom(up),
                    ListerKey::FindPrev => self.lister_find(false, true),
                    ListerKey::Encoding(e) => self.handle(Message::ListerEncoding(e)),
                    ListerKey::Wrap => self.handle(Message::ListerWrap),
                };
            }
            Message::ListerKey(_) => {}
            Message::Letter(k, c) => {
                return self.handle(if self.viewer() {
                    Message::ListerKey(k)
                } else {
                    Message::CmdType(c)
                });
            }
            Message::ListerPointer(x, y) => {
                if let Some(l) = &mut self.lister {
                    l.pointer = Some((x, y));
                    if l.dragging
                        && let Some(at) = l.pointer_pos()
                    {
                        match &mut l.sel {
                            Some(s) => s.head = at,
                            // Pressed before the first move was known: it starts here.
                            None => l.sel = Some(shagoff_core::lister::Selection::new(at)),
                        }
                    }
                }
            }
            Message::ListerPointerLeft => {
                if let Some(l) = &mut self.lister {
                    l.pointer = None;
                }
            }
            Message::ListerDoubleClick => {
                if let Some(l) = &mut self.lister {
                    l.dragging = false; // this click's release
                    l.select_at_pointer(false);
                    let row = l.pointer_pos().map(|(r, _)| r);
                    l.double = row.map(|r| (std::time::Instant::now(), r));
                }
            }
            Message::ListerPress => {
                // A third click soon after a double one on the same row: the whole row.
                if let Some(l) = &mut self.lister
                    && let Some((t, row)) = l.double.take()
                    && t.elapsed() < std::time::Duration::from_millis(500)
                    && l.pointer_pos().is_some_and(|(r, _)| r == row)
                {
                    l.select_at_pointer(true);
                    return Task::none();
                }
                if let Some(l) = &mut self.lister
                    && l.mode != shagoff_core::lister::Mode::Image
                {
                    let at = l.pointer_pos();
                    match (&mut l.sel, at) {
                        (Some(s), Some(at)) if self.mods.shift() => s.head = at,
                        (_, at) => l.sel = at.map(shagoff_core::lister::Selection::new),
                    }
                    l.dragging = true;
                }
            }
            Message::ListerRelease => {
                if let Some(l) = &mut self.lister {
                    l.dragging = false;
                    if l.sel.is_some_and(|s| s.is_empty()) {
                        l.sel = None; // a plain click: nothing selected
                    }
                }
            }
            Message::ListerScrolled(x, y, h) => {
                if let Some(l) = &mut self.lister {
                    (l.offset, l.height) = ((x, y), h);
                    if let Some(s) = l.edge() {
                        return self.lister_slide(s);
                    }
                }
            }
            Message::ListerFit => {
                if let Some(l) = &mut self.lister {
                    l.zoom = None;
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
            Message::ListerClose => {
                let window = self.lister.take().and_then(|l| l.window);
                if let Some(id) = window {
                    return cosmic::iced::window::close(id);
                }
            }
            Message::ListerWindow(w) => {
                if let Some(id) = self.lister.as_ref().and_then(|l| l.window) {
                    return match w {
                        ListerWin::Drag => cosmic::iced::window::drag(id),
                        ListerWin::Maximize => cosmic::iced::window::toggle_maximize(id),
                    };
                }
            }
            Message::WindowClosed(id) => {
                if self.lister.as_ref().is_some_and(|l| l.window == Some(id)) {
                    self.lister = None;
                }
            }
            Message::Linked(tab, made) => {
                let done = match made {
                    Ok((done, failed)) => {
                        if let Some(msg) = failed {
                            self.say(StatusKind::Error, msg);
                        }
                        done
                    }
                    Err(msg) => {
                        self.say(StatusKind::Error, msg);
                        return Task::none();
                    }
                };
                // Marks stay on the ones not linked, to try again. By id: the tab may have moved.
                if let Some(side) = self.side_of(tab)
                    && let Some(t) = (self.panes[side].items_mut().iter_mut()).find(|t| t.id == tab)
                {
                    t.panel.unmark(&done);
                }
                let reload = |app: &mut Self, s: usize| {
                    let to = app.panes[s].active().target();
                    app.reload(s, to, None)
                };
                return Task::batch([reload(self, 0), reload(self, 1)]);
            }
            Message::TreeRow(side, i) => {
                self.click_pane(side);
                if let Some(pt) = &mut self.panes[side].active_mut().tree {
                    pt.tree.cursor = i.min(pt.tree.rows.len().saturating_sub(1));
                    return Task::batch([self.reveal_active(side), self.tree_follow(side)]);
                }
            }
            Message::TreeRowToggle(side, i) => {
                self.click_pane(side);
                let hidden = self.panes[side].active().panel.show_hidden();
                if let Some(pt) = &mut self.panes[side].active_mut().tree {
                    pt.tree.cursor = i.min(pt.tree.rows.len().saturating_sub(1));
                    if let Some(dir) = pt.tree.toggle() {
                        return tree_read(pt.id, vec![dir], hidden, None);
                    }
                    // Closed: the cursor may be on the closed one now.
                    return Task::batch([self.reveal_active(side), self.tree_follow(side)]);
                }
            }
            Message::TreePick(i) => {
                if let Some(Dialog::Tree {
                    tree, hidden, id, ..
                }) = &mut self.dialog
                {
                    // On the row under the cursor: open / close it (Enter or «Go» goes there).
                    if tree.cursor == i {
                        if let Some(dir) = tree.toggle() {
                            return tree_read(*id, vec![dir], *hidden, None);
                        }
                        return Task::none();
                    }
                    tree.cursor = i.min(tree.rows.len().saturating_sub(1));
                }
            }
            Message::TreeFilled(from, read, select) => {
                // A panel's tree (Ctrl+F8): filled, its cursor kept in view, the other panel
                // following it.
                let at = (0..2).find_map(|s| {
                    (self.panes[s].items().iter())
                        .position(|t| t.tree.as_ref().is_some_and(|pt| pt.id == from))
                        .map(|i| (s, i))
                });
                if let Some((side, i)) = at {
                    let t = &mut self.panes[side].items_mut()[i];
                    let tab = t.id;
                    if let Some(PanelTree { tree, .. }) = &mut t.tree {
                        for (dir, names) in read {
                            tree.fill(&dir, names);
                        }
                        if let Some(p) = &select {
                            tree.select(p);
                        }
                    }
                    let shown = self.panes[side].active().id == tab;
                    return match shown {
                        true => Task::batch([self.reveal(side, tab), self.tree_follow(side)]),
                        false => Task::none(),
                    };
                }
                if let Some(Dialog::Tree { tree, id, .. }) = &mut self.dialog
                    && *id == from
                {
                    for (dir, names) in read {
                        tree.fill(&dir, names);
                    }
                    if let Some(p) = select {
                        tree.select(&p);
                    }
                    return list_snap(tree.cursor, tree.rows.len());
                }
            }
            Message::Tested(results) => {
                self.testing = false;
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("testing-archives")) {
                    self.status = None;
                }
                if results.is_empty() {
                    return Task::none(); // stopped before the first one
                }
                let n = results.len();
                let bad: Vec<String> = (results.iter())
                    .filter_map(|(a, r)| {
                        let name = a.file_name().unwrap_or_default().to_string_lossy();
                        let e = r.as_ref().err()?;
                        Some(format!("{name}: {}", crate::why::message(e)))
                    })
                    .collect();
                if bad.is_empty() {
                    let files: u64 = results.iter().filter_map(|(_, r)| r.as_ref().ok()).sum();
                    self.say(StatusKind::Info, fl!("tested-ok", n = n, files = files));
                } else {
                    let head = fl!("tested-bad", bad = bad.len(), n = n);
                    self.say(StatusKind::Error, format!("{head}\n{}", bad.join("\n")));
                }
            }
            Message::Summed(tab, r) => {
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("checksum-making")) {
                    self.status = None;
                }
                match r {
                    Ok(files) => {
                        let names: Vec<String> = (files.iter())
                            .map(|f| f.file_name().unwrap_or_default().to_string_lossy().into())
                            .collect();
                        let text = fl!("checksum-made", n = names.len(), names = names.join(", "));
                        self.say(StatusKind::Info, text);
                        // The cursor onto the (first) new file, the other panel just read again.
                        let reload = |app: &mut Self, s: usize, focus: Option<String>| {
                            let to = app.panes[s].active().target();
                            app.reload(s, to, focus)
                        };
                        let side = self.side_of(tab);
                        let focus = names.first().cloned();
                        return Task::batch([
                            reload(self, 0, focus.clone().filter(|_| side == Some(0))),
                            reload(self, 1, focus.filter(|_| side == Some(1))),
                        ]);
                    }
                    Err(e) if e.is_empty() => {} // stopped
                    Err(e) => self.say(StatusKind::Error, e),
                }
            }
            Message::Verified(results) => {
                self.testing = false;
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("checksum-verifying")) {
                    self.status = None;
                }
                if results.is_empty() {
                    return Task::none(); // stopped before the first one
                }
                let (mut ok, mut bad) = (0usize, Vec::new());
                for (sum, r) in &results {
                    let sum_name = sum.file_name().unwrap_or_default().to_string_lossy();
                    match r {
                        Err(e) => bad.push(format!("{sum_name}: {}", crate::why::message(e))),
                        Ok(rows) => {
                            for (name, c) in rows {
                                let name = name.display();
                                match c {
                                    Check::Ok => ok += 1,
                                    Check::NotFile => {
                                        bad.push(format!("{name}: {}", fl!("checksum-not-file")))
                                    }
                                    Check::Bad => {
                                        bad.push(format!("{name}: {}", fl!("checksum-bad")))
                                    }
                                    Check::Missing(k) => {
                                        let e = crate::why::text(&std::io::Error::from(*k));
                                        bad.push(format!("{name}: {e}"))
                                    }
                                }
                            }
                        }
                    }
                }
                if bad.is_empty() {
                    self.say(StatusKind::Info, fl!("checksum-ok", n = ok));
                } else {
                    let n = ok + bad.len();
                    let head = fl!("checksum-failed", bad = bad.len(), n = n);
                    self.say(StatusKind::Error, format!("{head}\n{}", bad.join("\n")));
                }
            }
            Message::Occupied(u) => {
                // Only its own "counting…": another job's Busy line stays.
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("counting")) {
                    self.status = None;
                }
                if let Some(u) = u {
                    let text = fl!(
                        "occupied",
                        size = crate::view::size_text(&self.config, u.bytes),
                        files = u.files,
                        dirs = u.dirs
                    );
                    self.say(StatusKind::Info, text);
                }
            }
            Message::TermWait => {} // `term_follow` runs after it
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
            Message::Escape(window)
                if self
                    .lister
                    .as_ref()
                    .is_some_and(|l| l.window == Some(window)) =>
            {
                // A dialog or a job of the main window is answered first.
                if self.dialog.is_some() || self.busy() {
                    return self.handle(Message::DialogCancel);
                }
                return self.lister_escape();
            }
            Message::Escape(window) if window != self.window_id() => {
                return self.update(Message::Surface(cosmic::surface::action::destroy_popup(
                    window,
                )));
            }
            // Esc during a header-edge drag: back to the widths before it.
            Message::Escape(_) if self.col_drag.is_some() => {
                self.col_drag = None;
                self.cols = self.config.columns;
            }
            Message::Escape(_) if matches!(&self.dialog, Some(Dialog::Toolbar(t)) if t.picker.is_some()) =>
            {
                return self.update(Message::ToolEdit(crate::toolbar::ToolMsg::PickClose));
            }
            // A drag of ours: the compositor does not cancel it on Esc, we end it.
            Message::Escape(_) if self.dragging => {
                self.dragging = false;
                self.right_drag = None;
                self.drop_hover = None;
                return cosmic::iced::runtime::dnd::end_dnd();
            }
            // Esc in the compare's editor: out of editing, the dialog stays.
            Message::Escape(_) if matches!(&self.dialog, Some(Dialog::Diff(d)) if d.editing.is_some()) =>
            {
                return self.handle(Message::DiffEditDone(false));
            }
            Message::Escape(_) => return self.handle(Message::DialogCancel),
            Message::DialogCancel => match self.dialog.take() {
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(Resolution::Cancel);
                }
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(ErrorChoice::Cancel);
                }
                Some(Dialog::Password { reply, .. }) => {
                    if let Some(r) = reply {
                        let _ = r.send(ErrorChoice::Cancel);
                    }
                }
                Some(Dialog::Connect { browsing, .. }) => {
                    if let Some(stop) = browsing {
                        stop.store(true, Ordering::Relaxed);
                    }
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
                    } else if self.viewer() {
                        return self.lister_escape();
                    } else if let Some((_, cancel)) = &self.connecting {
                        cancel.store(true, Ordering::Relaxed);
                    } else if (self.panes.iter())
                        .flat_map(|p| p.items())
                        .any(|t| t.branching.is_some())
                    {
                        // Every one under way: the hint does not say whose.
                        let mut back = Vec::new();
                        for side in 0..2 {
                            for (i, t) in self.panes[side].items_mut().iter_mut().enumerate() {
                                // Found, its rows being read: the dir as usual again.
                                if t.branching.is_some() && t.branch && t.results.is_some() {
                                    t.results = None;
                                    back.push((side, i, t.panel.cwd().to_path_buf()));
                                }
                                t.stop_branch();
                            }
                        }
                        self.say(StatusKind::Info, fl!("branch-stopped"));
                        let tasks: Vec<_> = (back.into_iter())
                            .map(|(side, i, cwd)| self.load_tab(side, i, cwd, None))
                            .collect();
                        return Task::batch(tasks);
                    } else if self.config.show_cmdline && !self.cmdline.is_empty() {
                        self.cmdline.clear();
                    } else {
                        self.panes[self.active].active_mut().panel.set_filter(None);
                    }
                }
            },
            Message::Op(event) => return self.on_job_event(event),
            Message::Pasted(Some((kind, paths))) if !paths.is_empty() && self.dialog.is_none() => {
                let op = match kind {
                    ClipKind::Copy => InputOp::Copy,
                    ClipKind::Cut => InputOp::Move,
                };
                // "": `cwd.join("")` ends with `/`, so `plan` always puts the files inside cwd.
                let before = (self.job.is_some(), self.queue.len());
                let task = self.start_transfer(op, self.active, paths, "");
                // Only when this paste started or queued a job: a refused one keeps the cut.
                if kind == ClipKind::Cut && (self.job.is_some(), self.queue.len()) != before {
                    return Task::batch([clip::clear(), task]);
                }
                return task;
            }
            Message::Pasted(_) => {}
            Message::DragStart(right) => {
                self.right_hold = None; // a drag, not a click: no mark, no menu
                self.right_drag = right;
                self.dragging = true;
            }
            Message::DragEnd => {
                self.dragging = false;
                self.drop_hover = None;
            }
            Message::DropAs(op) if matches!(self.dialog, Some(Dialog::DropMenu { .. })) => {
                if let Some(Dialog::DropMenu {
                    side, dir, sources, ..
                }) = self.dialog.take()
                {
                    self.dialog = Some(Dialog::Input {
                        op,
                        side,
                        sources,
                        input: dir_input(&dir),
                    });
                    return widget::text_input::focus(self.input_id.clone());
                }
            }
            Message::DropAs(_) => {} // the menu is gone already
            Message::DropRow(i) => {
                if let Some(Dialog::DropMenu { cursor, .. }) = &mut self.dialog {
                    *cursor = i;
                }
            }
            Message::DropAt(x, y) => {
                self.drop_at = Some(cosmic::iced::Point::new(x as f32, y as f32));
            }
            Message::DropHover(h) => self.drop_hover = h,
            Message::DropLeave(h) => {
                if self.drop_hover == Some(h) {
                    self.drop_hover = None;
                }
            }
            Message::Drop {
                side,
                dir,
                paths,
                mv,
            } => {
                self.drop_hover = None;
                // Any drop ends that drag, used or not: a later drop of the same files from
                // another program must not get the menu.
                let right_drag = self.right_drag.take();
                let at = self.drop_at.take().unwrap_or_default();
                let t = self.panes[side].active();
                // A job "In background" does not stop it: OK then says the job is running, as F5.
                // Search results have no one dir; a drop on a tab or a drive button names its own.
                if paths.is_empty()
                    || self.dialog.is_some()
                    || (dir.is_none() && t.results.is_some())
                {
                    return Task::none();
                }
                let dir = dir.unwrap_or_else(|| t.panel.cwd().to_path_buf());
                if shagoff_core::panel::drop_is_noop(&paths, &dir) {
                    return Task::none();
                }
                // The pane the files came from, if one shows them: its marks go when done, as F5.
                let parent = paths.first().and_then(|p| p.parent());
                let from = (0..2).find(|&s| Some(self.panes[s].active().panel.cwd()) == parent);
                let side = from.unwrap_or(side);
                // Our own right-button drag (the same files): ask, as TC's drop menu.
                if right_drag.is_some_and(|r| *r == paths) {
                    self.dialog = Some(Dialog::DropMenu {
                        side,
                        dir,
                        sources: paths,
                        at,
                        cursor: 0,
                    });
                    // A focused command line would take the menu's letters and Enter.
                    return unfocus();
                }
                // Shift as we see it: the compositor's chosen action ignores it (COSMIC keeps the
                // destination's preferred Copy), but the window keeps the keyboard while dragging.
                let mv = mv || self.mods.shift();
                // As F5 / F6 in TC: the same dialog, aimed at the dir the files were dropped on.
                self.dialog = Some(Dialog::Input {
                    op: if mv { InputOp::Move } else { InputOp::Copy },
                    side,
                    sources: paths,
                    input: dir_input(&dir),
                });
                return widget::text_input::focus(self.input_id.clone());
            }
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
            Message::JobPause => {
                if let Some(j) = &self.job {
                    j.control.pause.fetch_xor(true, Ordering::Relaxed);
                }
            }
            Message::QueueUp(i) => {
                if (1..self.queue.len()).contains(&i) {
                    self.queue.swap(i - 1, i);
                }
            }
            Message::Unqueue(i) => {
                if i < self.queue.len() {
                    self.queue.remove(i);
                    // The «В очереди: N» said when it was added is stale now.
                    self.status = None;
                }
            }
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
            Message::ListClear => match self.dialog.take() {
                Some(Dialog::List {
                    kind: ListKind::History,
                    side,
                    ..
                }) => self.panes[side].active_mut().history.clear(),
                Some(Dialog::List {
                    kind: ListKind::Commands,
                    ..
                }) => self.commands.clear(),
                other => self.dialog = other,
            },
            Message::OpenWithApps(side, files, r) => match r {
                // Asked in the background: a dialog opened meanwhile keeps its place.
                Ok(_) if self.dialog.is_some() => {}
                Ok(found) => {
                    self.open_with = files;
                    self.open_with_mime = found.mime;
                    self.dialog = Some(Dialog::List {
                        kind: ListKind::OpenWith,
                        side,
                        cursor: 0,
                        items: open_with_items(found.apps, false),
                    });
                }
                Err(e) => self.say(StatusKind::Error, fl!("open-failed", err = e)),
            },
            // "All programs…": the list shown is still the one that asked.
            Message::OpenWithAll(apps) => {
                if let Some(Dialog::List {
                    kind: ListKind::OpenWith,
                    cursor,
                    items,
                    ..
                }) = &mut self.dialog
                {
                    *items = open_with_items(apps, true);
                    *cursor = 0;
                    return scrollable::snap_to(
                        dialogs::list_scroll_id(),
                        scrollable::RelativeOffset {
                            x: None,
                            y: Some(0.0),
                        },
                    );
                }
            }
            Message::BranchListed {
                tab,
                id,
                root,
                paths,
            } => {
                // By id: the tab may have moved to the other panel meanwhile (Ctrl+U, a drag).
                let Some(side) = self.side_of(tab) else {
                    return Task::none();
                };
                let Some(i) = (self.panes[side].items().iter()).position(|t| t.id == tab) else {
                    return Task::none();
                };
                let t = &mut self.panes[side].items_mut()[i];
                // The tab went somewhere meanwhile (even back here), or asked again: not wanted
                // (and the status is someone else's by now).
                if t.branching != Some(id) {
                    return Task::none();
                }
                let t = &mut self.panes[side].items_mut()[i];
                if let Some(paths) = paths {
                    t.results = Some((root.clone(), Arc::new(paths)));
                    t.branch = true;
                    // The rows are read next, still under way (`Listed` ends it).
                    if self.panes[side].active().id == tab {
                        return self.reload(side, root, None);
                    }
                }
                self.panes[side].items_mut()[i].branching = None; // stopped, or read when shown
                if (self.status.as_ref()).is_some_and(|s| s.kind == StatusKind::Busy) {
                    self.status = None;
                }
            }
            Message::OpenWithDefault => {
                let Some(Dialog::List {
                    kind: ListKind::OpenWith,
                    cursor,
                    items,
                    ..
                }) = &self.dialog
                else {
                    return Task::none();
                };
                let Some(item) = items.get(*cursor).filter(|it| it.kind == Item::App) else {
                    return Task::none();
                };
                let (name, desktop, mime) = (
                    item.label.clone(),
                    item.path.clone(),
                    self.open_with_mime.clone(),
                );
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            openwith::set_default(&mime, &desktop).map(|()| name)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::OpenWithDefaultSet(r)),
                );
            }
            Message::OpenWithDefaultSet(r) => match r {
                Ok(app) => {
                    let mime = self.open_with_mime.clone();
                    self.say(
                        StatusKind::Info,
                        fl!("open-with-default-set", app = app, mime = mime),
                    );
                }
                Err(e) => self.say(StatusKind::Error, fl!("open-failed", err = e)),
            },
            // A Shift-press moves the button, it doesn't run it.
            Message::Tool(_) if self.mods.shift() || self.tool_drag.grab.is_some() => {}
            Message::Tool(i) => return self.run_tool(i),
            Message::ToolHover(at) => {
                self.tool_drag.hover(at, &mut self.config.toolbar);
            }
            Message::ToolLeft(i) => self.tool_drag.left(i),
            // Under a dialog the last button entered may still count as hovered.
            Message::ToolGrab if self.dialog.is_none() => self.tool_drag.grab(),
            Message::ToolGrab => {}
            Message::ToolRelease => self.tool_drop(),
            Message::ToolFailed(e) => self.say(StatusKind::Error, fl!("cmd-failed", error = e)),
            Message::Colors(m) => {
                if let Some(Dialog::Colors(c)) = &mut self.dialog {
                    c.update(m);
                }
            }
            Message::FontFilter(s) => {
                if let Some(Drawer::Settings(f)) = &mut self.drawer {
                    f.font_filter = s;
                }
            }
            Message::KeyEdit(m) => {
                if let Some(Dialog::Keys(k)) = &mut self.dialog {
                    k.update(m);
                }
            }
            Message::ToolEdit(m) => {
                if let Some(Dialog::Toolbar(t)) = &mut self.dialog {
                    let open = matches!(m, crate::toolbar::ToolMsg::PickOpen);
                    t.update(m);
                    if open && t.picker.is_some() {
                        return Task::perform(
                            async {
                                tokio::task::spawn_blocking(crate::toolbar::icon_names)
                                    .await
                                    .unwrap_or_default()
                            },
                            |v| {
                                cosmic::Action::App(Message::ToolEdit(
                                    crate::toolbar::ToolMsg::PickLoaded(v),
                                ))
                            },
                        );
                    }
                }
            }
            Message::Hot(m) => {
                if let Some(Dialog::Hotlist(h)) = &mut self.dialog {
                    let cwd = self.panes[h.side].active().panel.cwd().to_path_buf();
                    h.update(m, &cwd);
                }
            }
            // Quick search, viewer search or command line; not a dialog's or a settings field.
            // F2 only: Shift+F6 is Rename too.
            Message::FieldKey(Action::Rename)
                if !self.mods.shift()
                    && matches!(
                        self.dialog,
                        Some(Dialog::Input {
                            op: InputOp::Copy | InputOp::Move,
                            ..
                        })
                    ) =>
            {
                return self.handle(Message::DialogQueue);
            }
            // Ctrl+X passed on by a field. In the command line, on a line from the history
            // (Ctrl+E): the next command (TC). A typed line stays (the field cut its selection);
            // the other fields, the menu's «Cut» and the panel's Ctrl+X never step the history.
            Message::FieldKey(Action::ClipCut) => {
                if self.cmd_ready()
                    && self.commands.contains(&self.cmdline)
                    && let Some(next) = cmdline::next(&self.commands, &self.cmdline)
                {
                    self.cmdline = next;
                    return widget::text_input::move_cursor_to_end(self.cmd_id.clone());
                }
            }
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
                // The drop menu's hot letters (`&` in its items' names) pick at once.
                if let Some(Dialog::DropMenu { sources, .. }) = &self.dialog {
                    use shagoff_core::quicksearch::hotkey;
                    let key = c.to_lowercase().next();
                    let op = (dialogs::drop_ops(sources).iter().copied())
                        .find(|&op| hotkey(&dialogs::drop_label(op)).1.map(|(_, k)| k) == key);
                    return op.map_or_else(Task::none, |op| self.handle(Message::DropAs(op)));
                }
                // The tree: the next dir with that letter, as in a list.
                if let Some(Dialog::Tree { tree, .. }) = &mut self.dialog {
                    let names = tree.rows.iter().map(|n| n.name.as_str());
                    if let Some(i) = shagoff_core::quicksearch::next_with(names, tree.cursor, c) {
                        tree.cursor = i;
                        return list_snap(i, tree.rows.len());
                    }
                    return Task::none();
                }
                // A list dialog's letter key, as in TC's menus: the next item with that letter.
                if let Some(Dialog::List {
                    kind,
                    cursor,
                    items,
                    ..
                }) = &mut self.dialog
                {
                    use shagoff_core::quicksearch::{hotkey, next_with};
                    // Ctrl+D: a `&` hot letter picks its item at once; shared by several, it
                    // steps through them.
                    let hot: Vec<usize> = (items.iter().enumerate())
                        .filter(|(_, it)| {
                            *kind == ListKind::Hotlist
                                && hotkey(&it.label).1.map(|(_, k)| k) == c.to_lowercase().next()
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let [i] = hot[..] {
                        return self.pick(i);
                    }
                    if let Some(&i) = hot.iter().find(|&&i| i > *cursor).or(hot.first()) {
                        *cursor = i;
                        return list_snap(i, items.len());
                    }
                    let labels = items.iter().map(|it| it.label.as_str());
                    if let Some(i) = next_with(labels, *cursor, c) {
                        *cursor = i;
                        return list_snap(i, items.len());
                    }
                    return Task::none();
                }
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
                // Search results / a branch: not read again on every event (all of them stat'ed,
                // a million for a big tree; TC neither) — Ctrl+R does it.
                if self.job.is_none() && t.target() == t.panel.cwd() && t.results.is_none() {
                    let cwd = t.target();
                    return self.reload(side, cwd, None);
                }
            }
            Message::WindowWidth(window, w) if window == self.window_id() => self.win_w = w,
            Message::WindowWidth(..) => {}
            Message::Dismiss(None) => self.status = None,
            Message::Dismiss(Some(side)) => self.panes[side].active_mut().error = None,
            Message::SplitStart => (self.splitting, self.split_from) = (true, None),
            Message::SplitAt(window, x) if self.splitting && window == self.window_id() => {
                let from = *self.split_from.get_or_insert(x);
                if (x - from).abs() >= 3.0 {
                    self.split_from = Some(f32::NAN); // moved for real: follow from now on
                }
                if self.split_from.is_some_and(f32::is_nan) {
                    let (pad, gap) = crate::view::split_geometry(self.config.skin);
                    // Before the window's width is known: the two lists' (no Modern cards).
                    let total = match self.win_w {
                        w if w > 0.0 => w - 2.0 * pad - gap,
                        _ => self.panes[0].active().width + self.panes[1].active().width,
                    };
                    self.split = viewport::split_ratio(pad + gap / 2.0, total, x);
                }
            }
            Message::SplitAt(..) => {}
            Message::SplitEnd => self.splitting = false,
            Message::SplitReset => {
                (self.split, self.splitting) = (0.5, false);
            }
            Message::ColDragStart(side, edge) => self.col_drag = Some((side, edge, None)),
            Message::ColDrag(x) => {
                if let Some((_, edge, start)) = &mut self.col_drag {
                    let col = viewport::drag_target(*edge);
                    // On screen the widths are scaled with the text (`col_widths`); kept unscaled.
                    let scale = self.config.text_scale();
                    let &mut (x0, w0) = start.get_or_insert((x, self.cols[col] * scale));
                    let (col, w) = viewport::drag_col(*edge, w0, x0, x);
                    self.cols[col] = w / scale;
                }
            }
            // The main window's only (a context menu popup has its own coordinates).
            Message::TabDragAt(window, x) if window == self.window_id() => {
                let split = crate::view::pane_split(
                    self.config.skin,
                    self.panes[0].active().width,
                    self.config.active_frame,
                );
                self.pane_hover = Some(usize::from(x > split));
            }
            Message::TabDragAt(..) => {}
            Message::ColDragEnd => {
                if self.col_drag.take().is_some() {
                    self.save_columns();
                }
                // A tab held over the other pane: it goes there (TC), with Ctrl a copy. Not
                // while the Ctrl+Q viewer covers that pane.
                let over = self.pane_hover.take();
                let shown = self.viewer_shown();
                let covered = |to: usize| match shown {
                    None => false,
                    Some(None) => true,
                    Some(Some(s)) => s == to,
                };
                if let Some((from, _)) = self.tab_drag.take()
                    && over == Some(1 - from)
                    && !covered(1 - from)
                {
                    let action = match self.mods.control() {
                        true => Action::TabCopyOther,
                        false => Action::TabMoveOther,
                    };
                    return self.act(from, action);
                }
            }
            Message::PathPart(side, path) => {
                self.search = None;
                if self.busy() {
                    return Task::none();
                }
                let focus = panel::child_toward(self.panes[side].active().panel.cwd(), &path);
                self.active = side;
                return self.load(side, path, focus);
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
            Message::ConnectKeep(on) => {
                if let Some(Dialog::Connect { keep, .. }) = &mut self.dialog {
                    *keep = on;
                }
            }
            Message::Kept(r) => {
                if let Err(mount::Error::Failed(e)) = r {
                    self.say(StatusKind::Error, fl!("keyring-failed", error = e));
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
                        icon: String::new(),
                    }));
                }
            }
            Message::Made {
                side,
                cwd,
                name,
                first,
                made,
            } => match made {
                // Still that dir: the cursor goes onto the new one.
                Ok(()) if self.panes[side].active().target() == cwd => {
                    return self.reload(side, cwd, first);
                }
                Ok(()) => {}
                Err(e) => self.say(StatusKind::Error, fl!("mkdir-failed", path = name, err = e)),
            },
            Message::Terminals(list, default) => {
                (self.terminals, self.term_default) = (list, default)
            }
            Message::TermTx(tx) => {
                self.term_tx = Some(tx);
                self.term_start();
            }
            Message::TermStart => {
                self.term_ended = false;
                self.term_start();
            }
            Message::TermDragStart => {
                self.term_rows = self.config.term_rows; // a press let go unmoved saves the same
                self.term_drag = Some((f32::NAN, self.term_rows));
            }
            Message::TermDragAt(y) => {
                if let Some((from, rows)) = &mut self.term_drag {
                    if from.is_nan() {
                        *from = y;
                    }
                    let line = (f32::from(self.config.panel_size()) * 1.4).ceil();
                    let n = f32::from(*rows) + (*from - y) / line;
                    self.term_rows = n.round().clamp(3.0, 60.0) as u16;
                }
            }
            Message::TermDragEnd => {
                if self.term_drag.take().is_some() {
                    return self.set(Setting::TermRows(self.term_rows));
                }
            }
            Message::TermEvent(e) => {
                use crate::term::Outcome;
                let Some(pane) = &self.term else {
                    return Task::none();
                };
                match crate::term::on_event(pane, e) {
                    Outcome::None => {}
                    Outcome::Exit => {
                        (self.term, self.term_full, self.term_ended) = (None, false, true);
                    }
                    Outcome::Copy(text) => return cosmic::iced::clipboard::write(text),
                }
            }
            Message::Gvfs(drives) => {
                self.gvfs_busy = false;
                if drives != self.gvfs {
                    self.gvfs = drives;
                    self.set_drives();
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
                    let mut keep = Task::none();
                    if let Some(Dialog::Connect {
                        browsing,
                        password,
                        keep: on,
                        ..
                    }) = &self.dialog
                    {
                        if let Some(stop) = browsing {
                            stop.store(true, Ordering::Relaxed); // a browse no one will see
                        }
                        if *on && !password.is_empty() {
                            let (url, password) = (url.clone(), password.clone());
                            let no_realm = fl!("keyring-no-realm");
                            keep = blocking(
                                move || {
                                    use shagoff_core::keyring::{self, KeepError};
                                    keyring::save(&url, &password).map_err(|e| {
                                        mount::Error::Failed(match e {
                                            KeepError::NoRealm => no_realm,
                                            KeepError::Failed(e) => e,
                                        })
                                    })
                                },
                                Message::Kept,
                            );
                        }
                        self.dialog = None;
                    }
                    return Task::batch([keep, self.handle(Message::Mounted(side, result))]);
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
            Message::ConnectBrowse(place) => {
                if let Some(Dialog::Connect {
                    url,
                    password,
                    found,
                    browsing,
                    at,
                    note,
                    ..
                }) = &mut self.dialog
                {
                    if let Some(stop) = browsing.take() {
                        stop.store(true, Ordering::Relaxed);
                        // The button again while looking: only stop.
                        if place.is_none() {
                            return Task::none();
                        }
                    }
                    *at = place.unwrap_or_else(|| mount::NETWORK.to_string());
                    found.clear();
                    *note = None;
                    let stop = Arc::new(AtomicBool::new(false));
                    *browsing = Some(stop.clone());
                    // The password was typed for the address's host: no one else gets it.
                    let password = if mount::host(url) == mount::host(at) {
                        password.clone()
                    } else {
                        String::new()
                    };
                    let (place, mine) = (at.clone(), stop.clone());
                    return blocking(
                        move || mount::browse(&place, &password, &stop),
                        move |r| Message::Browsed(mine, r),
                    );
                }
            }
            Message::Browsed(stop, r) => {
                if let Some(Dialog::Connect {
                    found,
                    browsing,
                    note,
                    ..
                }) = &mut self.dialog
                {
                    // Stopped or replaced by another: not wanted.
                    if !browsing.as_ref().is_some_and(|b| Arc::ptr_eq(b, &stop)) {
                        return Task::none();
                    }
                    *browsing = None;
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
            Message::Config(c) => return self.apply_config(*c),
            Message::CloseDrawer => self.set_drawer(None),
            Message::FocusNext => return cosmic::iced::widget::operation::focus_next(),
            Message::Setting(s) => return self.set(s),
            Message::CheckUpdate => {
                use shagoff_core::update::{Status, check};
                self.update_check = Some(Status::Checking);
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| check(env!("CARGO_PKG_VERSION")))
                            .await
                            .unwrap_or_else(|e| Status::Failed(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::UpdateChecked(r)),
                );
            }
            Message::UpdateChecked(s) => self.update_check = Some(s),
            Message::OpenUrl(url) => {
                if let Err(e) = spawn_detached(&["xdg-open".into(), url.into()]) {
                    log::warn!("xdg-open: {e}");
                }
            }
            Message::CloseRequested(id) if id == self.window_id() => {
                return self.handle(Message::Quit);
            }
            Message::CloseRequested(_) => {}
            // Confirmed, but the worker hangs in blocking I/O (a dead network mount): × again quits.
            Message::Quit if self.quitting => return self.handle(Message::Exit),
            // Unsaved copied blocks in the comparison: the first × only warns, as Esc.
            Message::Quit
                if matches!(&self.dialog, Some(Dialog::Diff(d))
                    if (d.dirty.0 || d.dirty.1) && !d.confirm_close) =>
            {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    d.confirm_close = true;
                }
            }
            Message::Quit if self.job.is_some() || !self.queue.is_empty() => {
                // A job's question steps aside (its reply kept); any other dialog gives way.
                if let Some(
                    d @ (Dialog::Conflict { .. } | Dialog::Error { .. } | Dialog::Password { .. }),
                ) = self.dialog.take()
                {
                    self.held = Some(d);
                }
                self.dialog = Some(Dialog::ConfirmExit);
            }
            Message::Quit => return self.handle(Message::Exit),
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
        // The divider is written once it is let go, not on every move.
        if self.splitting {
            return;
        }
        // Off: the histories live for this run only, and what was saved is wiped.
        let keep = |v: &Vec<String>| {
            if self.config.keep_history {
                v.clone()
            } else {
                Vec::new()
            }
        };
        let state = State {
            panes: [0, 1].map(|s| PaneState {
                // One locked with dir changes allowed opens in its dir next time.
                tabs: self.panes[s]
                    .items()
                    .iter()
                    .map(|t| {
                        t.home
                            .clone()
                            .unwrap_or_else(|| t.panel.cwd().to_path_buf())
                    })
                    .collect(),
                roam: self.panes[s]
                    .items()
                    .iter()
                    .map(|t| t.home.is_some())
                    .collect(),
                active: self.panes[s].active_index(),
                locked: self.panes[s].items().iter().map(|t| t.locked).collect(),
                brief: self.panes[s].items().iter().map(|t| t.brief).collect(),
                thumbs: self.panes[s].items().iter().map(|t| t.thumbs).collect(),
                names: (self.panes[s].items().iter())
                    .map(|t| t.name.clone().unwrap_or_default())
                    .collect(),
                history: if self.config.keep_history {
                    self.panes[s]
                        .items()
                        .iter()
                        .map(|t| t.history.clone())
                        .collect()
                } else {
                    Vec::new()
                },
            }),
            active: self.active,
            find: self.find.clone(),
            sync: self.sync.clone(),
            split: self.split,
            commands: keep(&self.commands),
            masks: keep(&self.masks),
            other_cmds: keep(&self.other_cmds),
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

    /// The file list's font and size.
    pub fn panel_font(&self) -> dialogs::ListFont {
        (self.face, self.config.panel_size())
    }

    /// File row height of the current skin and text size; every scroll computation uses it.
    pub fn row_h(&self) -> f32 {
        self.config.row_h()
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A path inside a nested archive's temp copy as where that archive lies (`outer.zip/inner.zip/…`).
    pub fn shown(&self, p: &Path) -> PathBuf {
        for (tmp, back) in &self.nested {
            if let Ok(rest) = p.strip_prefix(tmp) {
                let name = tmp.file_name().unwrap_or_default();
                return self.shown(&back.join(name).join(rest));
            }
        }
        p.to_path_buf()
    }

    /// `shown`'s way back: a path through a nested archive (a click on the path line, `cd`) to its
    /// temp copy; the deepest nesting that matches.
    fn real(&self, p: &Path) -> PathBuf {
        (self.nested.keys())
            .filter_map(|tmp| {
                let root = self.shown(tmp);
                let rest = p.strip_prefix(&root).ok()?;
                Some((root.components().count(), tmp.join(rest)))
            })
            .max_by_key(|(n, _)| *n)
            .map_or_else(|| p.to_path_buf(), |(_, real)| real)
    }

    /// Go to `path` in the active tab (leaves search results).
    fn load(&mut self, side: usize, path: PathBuf, focus: Option<String>) -> Task<Message> {
        let path = self.real(&path);
        // A locked tab keeps its dir: going elsewhere happens in a new tab next to it (TC).
        if path != self.panes[side].active().panel.cwd() {
            self.leave_locked(side);
        }
        let t = self.panes[side].active_mut();
        t.results = None;
        t.stop_branch(); // a branch read for the dir left (or re-entered) is not wanted
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

    /// The columns' widths on screen: as kept (for the default text size) grown or shrunk with
    /// the panel's text, so a bigger font does not cut the dates (162).
    pub fn col_widths(&self) -> [f32; 4] {
        self.cols.map(|w| w * self.config.text_scale())
    }

    /// A tab locked with dir changes allowed that is no longer the shown one is read in its dir
    /// again (TC: back to the locked dir on a tab change).
    fn roam_back(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for side in 0..2 {
            let now = self.panes[side].active().id;
            let was = std::mem::replace(&mut self.shown_tabs[side], now);
            let items = self.panes[side].items();
            let away = (items.iter().position(|t| t.id == was && was != now))
                .and_then(|i| Some((i, items[i].home.clone()?)))
                .filter(|(i, home)| items[*i].target() != *home);
            if let Some((i, home)) = away {
                tasks.push(self.load_tab(side, i, home, None));
            }
        }
        Task::batch(tasks)
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
    /// Ctrl+F8 on, off, and the keys of a tab showing the tree. `None`: not a tree (the action goes
    /// on as usual). File operations do nothing there: the files under it are not on screen.
    fn tree_act(&mut self, side: usize, action: Action) -> Option<Task<Message>> {
        if action == Action::TreePanel {
            let t = self.panes[side].active_mut();
            if let Some(pt) = t.tree.take() {
                t.offset = pt.files_offset; // the file list where it was
                return Some(Task::batch([
                    self.restore_scroll(side),
                    self.reveal_active(side),
                ]));
            }
            return Some(self.tree_open(side));
        }
        let hidden = self.panes[side].active().panel.show_hidden();
        let tab = self.panes[side].active().id;
        let pt = self.panes[side].active_mut().tree.as_mut()?;
        let (id, tree) = (pt.id, &mut pt.tree);
        const PAGE: isize = 10;
        match action {
            Action::Up => tree.move_cursor(-1),
            Action::Down => tree.move_cursor(1),
            Action::PageUp => tree.move_cursor(-PAGE),
            Action::PageDown => tree.move_cursor(PAGE),
            Action::Home => tree.move_cursor(-isize::MAX / 2),
            Action::End => tree.move_cursor(isize::MAX / 2),
            Action::Right => {
                if let Some(dir) = tree.step_in() {
                    return Some(tree_read(id, vec![dir], hidden, None));
                }
            }
            Action::Left | Action::Parent => tree.step_out(),
            Action::QuickSearch(c) => {
                let names = tree.rows.iter().map(|n| n.name.as_str());
                if let Some(i) = shagoff_core::quicksearch::next_with(names, tree.cursor, c) {
                    tree.cursor = i;
                }
            }
            // The files of the dir chosen: in the other panel already.
            Action::Enter => {
                self.active = 1 - side;
                return Some(Task::none());
            }
            // About the files under the tree, not on screen: nothing. The rest (tabs, menus,
            // views, the terminal…) as usual.
            a if on_files(a) => return Some(Task::none()),
            _ => return None,
        }
        Some(Task::batch([
            self.reveal(side, tab),
            self.tree_follow(side),
        ]))
    }

    /// Ctrl+F8 on (or the tab's dir changed under it): the tree from `/`, opened down to the
    /// tab's dir (inside an archive: the archive's own; a nested one: where the outer lies).
    fn tree_open(&mut self, side: usize) -> Task<Message> {
        use shagoff_core::tree::Tree;
        let hidden = self.panes[side].active().panel.show_hidden();
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let cwd = self.shown(&cwd);
        let here = match archive::split_path(&cwd) {
            Some((a, _)) => a.parent().map_or(cwd.clone(), Path::to_path_buf),
            None => cwd,
        };
        let root = PathBuf::from("/");
        let id = self.next_id();
        let t = self.panes[side].active_mut();
        let files_offset = t.tree.take().map_or(t.offset, |pt| pt.files_offset);
        t.tree = Some(PanelTree {
            id,
            tree: Tree::new(&root),
            files_offset,
        });
        t.offset = 0.0;
        let dirs = shagoff_core::tree::chain(&root, &here);
        Task::batch([
            self.restore_scroll(side),
            tree_read(id, dirs, hidden, Some(here)),
        ])
    }

    /// `reveal` for the active tab of `side`.
    fn reveal_active(&mut self, side: usize) -> Task<Message> {
        let tab = self.panes[side].active().id;
        self.reveal(side, tab)
    }

    /// The other panel to the dir under tab `side`'s tree cursor (TC), unless it is there.
    fn tree_follow(&mut self, side: usize) -> Task<Message> {
        let Some(dir) = (self.panes[side].active().tree.as_ref())
            .and_then(|pt| pt.tree.current())
            .map(|n| n.path.clone())
        else {
            return Task::none();
        };
        // A locked tab there would open a new tab at every step: it stays.
        let other = self.panes[1 - side].active();
        if other.target() == dir || other.locked {
            return Task::none();
        }
        self.load(1 - side, dir, None)
    }

    /// The panel holding tab `tab` now (tabs move: Ctrl+U, a drag), `None` once it is closed.
    fn side_of(&self, tab: u64) -> Option<usize> {
        (0..2).find(|&s| self.panes[s].items().iter().any(|t| t.id == tab))
    }

    /// Scan `path` for the active tab of `side` in the background; the result lands in `Message::Listed`.
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
        t.pending_at = Instant::now();
        // Going anywhere else leaves the search results.
        if t.results.as_ref().is_some_and(|(root, _)| *root != path) {
            t.results = None;
        }
        let results = t.results.as_ref().map(|(_, r)| r.clone());
        // Ctrl+B's second step: Esc stops it as it stops the first.
        let stop = match t.branching {
            Some(_) => t.branch_stop.clone(),
            None => Arc::default(),
        };
        let tab = t.id;
        let (show_hidden, sort) = (t.panel.show_hidden(), t.panel.sort());
        // The dir shown read again (restored at startup, a watcher event, Ctrl+R) and gone: up to
        // one that is there, as TC. Found by the background read — never a `stat` on the UI
        // thread, a dead mount would hang it. A step elsewhere into a gone dir stays an error.
        let resolve = (path == t.panel.cwd()).then(|| self.home.clone());
        Task::perform(
            async move {
                let (result, space, path) = tokio::task::spawn_blocking(move || {
                    let p = match resolve {
                        Some(home) => session::existing_dir(&path, &home),
                        None => path,
                    };
                    let mut result = read_listing(&p, show_hidden, results.as_deref(), &stop);
                    // Sorted here, not on the UI thread; the panel only checks the order.
                    // ponytail: counted dir sizes are not known here; a Size sort with them is
                    // redone by the panel.
                    if let Ok(entries) = &mut result {
                        shagoff_core::sort::sort_entries(entries, sort, &HashMap::new());
                    }
                    let space = drives::space(&p);
                    (result, space, p)
                })
                .await
                .unwrap_or_else(|e| (Err(e.to_string()), None, PathBuf::new()));
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

    /// A click in pane `side`: as a key there (and Tab away from the other) — the error shown
    /// in the pane left and in this one goes, quick search closes.
    fn click_pane(&mut self, side: usize) {
        self.search = None;
        // The terminal does not see a click its rows took first: the keys come back to the panels.
        self.term_blur = self.term.is_some();
        for s in [self.active, side] {
            self.panes[s].active_mut().error = None;
        }
        self.active = side;
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
        if let Some(task) = self.tree_act(side, action) {
            return task;
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
            Action::OpenWith => return self.open_with(side),
            Action::Help | Action::About | Action::Settings | Action::Donate => {
                self.toggle_drawer(action);
                return Task::none();
            }
            // Nothing set: the settings (left open if they are already).
            Action::F12 if self.config.f12.cmd.trim().is_empty() => {
                if !matches!(self.drawer, Some(Drawer::Settings(_))) {
                    self.toggle_drawer(Action::Settings);
                }
                return Task::none();
            }
            Action::F12 => return self.run_button(self.config.f12.clone()),
            Action::ToggleHumanSizes => {
                return self.set(Setting::HumanSizes(!self.config.human_sizes()));
            }
            Action::ToggleDividerLine => {
                return self.set(Setting::DividerLine(!self.config.divider_line));
            }
            Action::ToggleActiveFrame => {
                return self.set(Setting::ActiveFrame(!self.config.active_frame));
            }
            Action::ToggleTerminal => {
                let on = !self.config.term_builtin;
                self.term_want_focus = on;
                let task = self.set(Setting::TermBuiltin(on));
                // Hidden while it had the keys: back to the panels.
                return match on {
                    true => task,
                    false => Task::batch([task, unfocus()]),
                };
            }
            Action::TermFull => {
                if !self.config.term_builtin {
                    return Task::none();
                }
                if self.term.is_none() {
                    self.term_ended = false;
                    self.term_start();
                }
                self.term_full = !self.term_full && self.term.is_some();
                // Over the whole height: its keys; back: the panels'.
                return match self.term_full {
                    true => cosmic::iced::runtime::widget::operation::focus(self.term_id.clone()),
                    false => unfocus(),
                };
            }
            Action::PickShell => {
                let (items, cursor) = self.shell_items();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Shells,
                    side,
                    cursor,
                    items,
                });
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
        // Said before its dialog (the job would be refused anyway: `changes_nested`).
        if self.nested_change(side, action) {
            self.say(StatusKind::Error, fl!("nested-read-only"));
            return Task::none();
        }
        if action == Action::Hotlist {
            self.hot_sub.clear(); // Ctrl+D always opens at the top
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
        let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
        let (brief, rows) = (t.brief, rows as isize);
        let (thumbs, across) = (t.thumbs, t.thumb_cols() as isize);
        let page = if brief {
            rows * cols as isize
        } else if thumbs {
            viewport::page_rows(crate::thumbs::cell_h(row_h), t.height) as isize * across
        } else {
            viewport::page_rows(row_h, t.height) as isize
        };
        let tab = t.id;
        let target = t.target();
        let loading = target != t.panel.cwd();
        // A double Enter must not open a row of the dir being left; a read hung on a dead mount
        // must not hold Enter for good: after a moment the rows on screen are what it opens.
        let stepping = loading && t.pending_at.elapsed() < ENTER_WAIT;
        let panel = &mut t.panel;
        match action {
            Action::SwitchPane => {
                self.active = 1 - self.active;
                return Task::none();
            }
            Action::Up if thumbs => panel.move_cursor(-across),
            Action::Down if thumbs => panel.move_cursor(across),
            Action::Left if thumbs => panel.move_cursor(-1),
            Action::Right if thumbs => panel.move_cursor(1),
            Action::Up => panel.move_cursor(-1),
            Action::Down => panel.move_cursor(1),
            Action::Left if brief => panel.move_cursor(-rows),
            Action::Right if brief => panel.move_cursor(rows),
            Action::Left | Action::Right => {}
            Action::ConfigureColors => {
                let mut edit = crate::colorrules::ColorEdit::new(self.config.colors.clone());
                edit.panel = self.config.panel.clone();
                self.dialog = Some(Dialog::Colors(Box::new(edit)));
                return Task::none();
            }
            Action::OpenTerminal => {
                self.open_terminal();
                return Task::none();
            }
            Action::PickTerminal => {
                let (items, cursor) = self.terminal_items();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Terminals,
                    side,
                    cursor,
                    items,
                });
                return Task::none();
            }
            Action::ConfigureKeys => {
                let list = self.config.hotkeys.clone();
                self.dialog = Some(Dialog::Keys(Box::new(crate::hotkeys::KeyEdit::new(list))));
                return Task::none();
            }
            Action::ConfigureToolbar => {
                let list = self.config.toolbar.clone();
                self.dialog = Some(Dialog::Toolbar(Box::new(crate::toolbar::ToolEdit::new(
                    list,
                ))));
                return Task::none();
            }
            Action::ResetColumns => {
                self.cols = Config::default().columns;
                self.save_columns();
                return Task::none();
            }
            Action::ViewBrief | Action::ViewFull | Action::ViewThumbs => {
                let t = self.panes[side].active_mut();
                t.brief = action == Action::ViewBrief;
                t.thumbs = action == Action::ViewThumbs;
                return Task::batch([self.reveal(side, tab), self.restore_scroll(side)]);
            }
            Action::PageUp => panel.move_cursor(-page),
            Action::PageDown => panel.move_cursor(page),
            Action::Home => panel.cursor_home(),
            Action::End => panel.cursor_end(),
            // 100 000 rows take ~0.1 s to sort: a big list is sorted by a background read.
            Action::Sort(key) if panel.entries().len() > SORT_HERE_MAX => {
                panel.set_sort_key(key);
                return self.reload(side, target, None);
            }
            Action::Sort(key) => panel.set_sort(key),
            // The rows on screen belong to the dir being left: entering one would undo the navigation.
            // Shift+Enter, no line typed: the file under the cursor in the terminal (a dir: Enter).
            Action::RunTerminal => {
                let cwd = panel.cwd().to_path_buf();
                let file = (panel.current())
                    .filter(|e| e.kind == shagoff_core::listing::Kind::File)
                    .map(|e| e.name.clone());
                let Some(name) = file else {
                    return self.act(side, Action::Enter);
                };
                if archive::within(&cwd) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
                    return Task::none();
                }
                let prefix = self.terminal_prefix();
                let line = cmdline::quote(&format!("./{name}"));
                if let Err(e) = spawn_in(&cmdline::argv(&line, &cwd, Some(&prefix)), &cwd) {
                    self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                }
            }
            Action::Enter if stepping => {}
            Action::Enter => {
                // ".." at a nested archive's root: back into the outer one, as Backspace.
                if panel.current().is_some_and(|e| e.name == PARENT)
                    && self.nested.contains_key(panel.cwd())
                {
                    return self.act(side, Action::Parent);
                }
                if let Some((path, focus)) = panel.enter_path() {
                    return self.load(side, path, focus);
                }
                let cwd = panel.cwd().to_path_buf();
                let current = panel.current().map(|e| (e.os_name.clone(), e.name.clone()));
                if let Some((os_name, name)) = current {
                    let (dir, own) = located(&cwd, &os_name);
                    match archive::split_path(&dir) {
                        Some((arc, inner)) => {
                            let os_name = own;
                            let how = match Format::detect(&name).is_some_and(Format::is_tree) {
                                true => How::Browse,
                                false => How::Run(Vec::new(), &["xdg-open"]),
                            };
                            return self.open_from_archive(side, arc, inner, os_name, how);
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
                    self.say(
                        StatusKind::Error,
                        fl!("open-failed", err = crate::why::text(&err)),
                    );
                }
            }
            // From where the tab is going, so quick Backspaces on a slow fs are not lost.
            Action::Parent => {
                // The root of an archive opened from inside another: back into that one.
                if let Some(back) = self.nested.get(&target).cloned() {
                    let name = target.file_name().map(|n| n.to_string_lossy().into_owned());
                    return self.load(side, back, name);
                }
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
                    let stop = self.panes[side].active().counting.clone();
                    let count = count_dirs(tab, cwd, vec![name], stop);
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
                let stop = self.panes[side].active().counting.clone();
                return count_dirs(tab, cwd, names, stop);
            }
            Action::MarkDown => panel.toggle_mark_and_move(1),
            Action::MarkUp => panel.toggle_mark_and_move(-1),
            Action::Invert => panel.invert(),
            Action::RestoreMarks => panel.restore_marks(),
            Action::ShowAll => panel.set_show(Show::All),
            Action::ShowPrograms => panel.set_show(Show::Programs),
            Action::MarkExt => panel.mark_ext(true),
            Action::UnmarkExt => panel.mark_ext(false),
            // TC: the dir / archive under the cursor (`..`: the parent) in that panel; on a file,
            // this panel's dir.
            Action::OpenLeft | Action::OpenRight => {
                let to = usize::from(action == Action::OpenRight);
                let path = match panel.drop_target(panel.cursor()) {
                    Some(p) => p,
                    // On a file in this very panel: nothing (reading its dir again would drop
                    // search results / a branch).
                    None if to == side => return Task::none(),
                    None => panel.cwd().to_path_buf(),
                };
                return self.load(to, path, None);
            }
            Action::Occupied => {
                // Nothing marked, on `..`: the dir itself (TC).
                let mut targets = panel.targets();
                if targets.is_empty() {
                    targets.push(panel.cwd().to_path_buf());
                }
                if loading || targets.iter().any(|p| inside_archive(p)) {
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("counting"));
                // The tab's: leaving the dir stops it, as Space's counts.
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            shagoff_core::props::usage(&targets, &stop)
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    |u| cosmic::Action::App(Message::Occupied(u)),
                );
            }
            Action::TestArchives => {
                // One at a time; not on the rows of a dir being left.
                if self.testing || loading {
                    return Task::none();
                }
                // Dirs by the panel's rows, not a `stat` here (a dir may be named `x.zip`).
                let dirs: std::collections::HashSet<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| panel.cwd().join(&e.os_name))
                    .collect();
                let archives: Vec<PathBuf> = (panel.targets().into_iter())
                    .filter(|p| !inside_archive(p) && !dirs.contains(p))
                    .filter(|p| {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        archive::Format::detect(&name).is_some()
                    })
                    .collect();
                if archives.is_empty() {
                    self.say(StatusKind::Error, fl!("test-no-archives"));
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("testing-archives"));
                self.testing = true;
                // The tab's: leaving the dir stops it.
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            // Stopped (`Interrupted`): no more, and that one is not "damaged".
                            (archives.into_iter())
                                .map_while(|a| match archive::test(&a, &stop) {
                                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => None,
                                    r => Some((a, r.map_err(|e| crate::why::text(&e)))),
                                })
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |r| cosmic::Action::App(Message::Tested(r)),
                );
            }
            Action::ChecksumVerify => {
                if self.testing || loading {
                    return Task::none();
                }
                // Dirs by the panel's rows, not a `stat` here (as Alt+Shift+F9).
                let dirs: std::collections::HashSet<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| panel.cwd().join(&e.os_name))
                    .collect();
                let sums: Vec<PathBuf> = (panel.targets().into_iter())
                    .filter(|p| !inside_archive(p) && !dirs.contains(p))
                    .filter(|p| {
                        checksum::is_sum_file(&p.file_name().unwrap_or_default().to_string_lossy())
                    })
                    .collect();
                if sums.is_empty() {
                    self.say(StatusKind::Error, fl!("checksum-no-files"));
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("checksum-verifying"));
                self.testing = true;
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            (sums.into_iter())
                                .map_while(|s| match checksum::verify(&s, &stop) {
                                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => None,
                                    r => Some((s, r.map_err(|e| crate::why::text(&e)))),
                                })
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |r| cosmic::Action::App(Message::Verified(r)),
                );
            }
            Action::TreePanel => {} // `tree_act`
            Action::TreeDialog => {
                let hidden = panel.show_hidden();
                let cwd = panel.cwd().to_path_buf();
                // A dir on disk: inside an archive, the archive's own (a nested one: where the
                // outer archive lies, not its temp copy).
                let cwd = self.shown(&cwd);
                let here = match archive::split_path(&cwd) {
                    Some((a, _)) => a.parent().map_or(cwd.clone(), Path::to_path_buf),
                    None => cwd,
                };
                let root = PathBuf::from("/");
                let id = self.next_id();
                self.dialog = Some(Dialog::Tree {
                    side,
                    id,
                    tree: shagoff_core::tree::Tree::new(&root),
                    hidden,
                });
                let dirs = shagoff_core::tree::chain(&root, &here);
                return tree_read(id, dirs, hidden, Some(here));
            }
            Action::ContextMenu => {
                let ctx = match panel.current() {
                    Some(e) if e.name != PARENT => crate::menu::Ctx::Entry {
                        dir: e.is_dir(),
                        archive: archive::Format::detect(&e.name).is_some_and(|f| f.is_tree()),
                    },
                    _ => crate::menu::Ctx::Dir,
                };
                let items = (crate::menu::context_rows(ctx).into_iter())
                    .map(|(label, act)| ListItem {
                        label,
                        path: PathBuf::new(),
                        kind: act.map_or(Item::Sep, Item::Act),
                        icon: String::new(),
                    })
                    .collect();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Context,
                    side,
                    cursor: 0,
                    items,
                });
                return Task::none();
            }
            Action::SelectAll => panel.mark_all(true),
            Action::UnselectAll => panel.mark_all(false),
            Action::ToggleHidden => {
                let on = !self.config.show_hidden;
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_show_hidden(h, on) {
                            self.config_not_saved(&e);
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
                let current = current.map(|os| located(&cwd, &os));
                if let Some((os_name, (arc, inner))) =
                    current.and_then(|(dir, own)| Some((own, archive::split_path(&dir)?)))
                {
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
                    if let Some(why) = unopenable(&file, &name) {
                        self.say(StatusKind::Error, why);
                        return Task::none();
                    }
                    let argv = if action == Action::View {
                        launch::command(&self.config.viewer, &["xdg-open"], &file)
                    } else {
                        launch::command(&self.config.editor, &["cosmic-edit"], &file)
                    };
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(
                            StatusKind::Error,
                            fl!("open-failed", err = crate::why::text(&err)),
                        );
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
            | Action::F12
            | Action::ToggleDividerLine
            | Action::ToggleHumanSizes
            | Action::ToggleActiveFrame
            | Action::ToggleTerminal
            | Action::TermFull
            | Action::PickShell
            | Action::Donate
            | Action::OpenWith
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
            Action::Branch | Action::BranchSel => return self.branch(side, action),
            Action::QuickView => {
                match self.lister.as_deref() {
                    Some(l) if l.quick.is_some() => self.lister = None,
                    Some(_) => {} // the full viewer is up: not from there
                    None => {
                        let mut l = Lister::new(side, String::new(), 0, self.config.lister_size());
                        // Nothing yet: `quick_follow` loads it.
                        l.quick = Some((PathBuf::new(), 0, std::time::UNIX_EPOCH));
                        self.lister = Some(Box::new(l));
                    }
                }
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
            Action::ClipCopy | Action::ClipCut => {
                let paths = panel.targets();
                if !paths.is_empty() {
                    let kind = if action == Action::ClipCut {
                        ClipKind::Cut
                    } else {
                        ClipKind::Copy
                    };
                    // Through an archive (a stat, not the name: `backup.zip/` may be a real dir).
                    let inner =
                        (paths.iter()).any(|p| p.parent().and_then(archive::split_path).is_some());
                    return clip::put(kind, &paths, inner);
                }
            }
            Action::ClipPaste => return clip::take(),
            Action::Copy
            | Action::CopySame
            | Action::Link
            | Action::Properties
            | Action::Move
            | Action::Rename
            | Action::MultiRename
            | Action::Pack
            | Action::Unpack
            | Action::ChecksumCreate
            | Action::FindFiles
            | Action::SyncDirs
            | Action::Mkdir
            | Action::NewFile
            | Action::Delete
            | Action::DeletePermanent => {}
            // Opened by `dialog_for` above.
            Action::SelectGroup | Action::UnselectGroup | Action::ShowMask => {}
            // Also the drive list: a stick plugged in shows up (TC rereads drives too).
            Action::Reload => {
                // Where the tab is going (results fed, a dir entered, not listed yet), not where
                // it was: that would drop the results / undo the step.
                let cwd = self.panes[side].active().target();
                // Pictures are known by mtime: a chmod or `cp -p` is seen only on Ctrl+R.
                self.thumbs.clear();
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
                // TC: in the sort of the tab it was opened from.
                let sort = self.panes[side].active().panel.sort();
                let mut tab = self.new_tab(path.clone());
                tab.panel.set_sort_order(sort);
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
            Action::TabRoam => {
                let t = self.panes[side].active_mut();
                t.home = match t.home {
                    Some(_) => None,
                    None => Some(t.panel.cwd().to_path_buf()),
                };
                t.locked = false;
                return Task::none();
            }
            Action::TabLock => {
                let t = self.panes[side].active_mut();
                t.locked = !t.locked;
                t.home = None;
                // A scan into another dir would still land and move it.
                if t.locked && t.target() != t.panel.cwd() {
                    t.pending = None;
                }
                return Task::none();
            }
            Action::CloseOtherTabs => {
                self.panes[side].retain(Tab::pinned);
                return Task::none();
            }
            Action::NewTab => {
                let id = self.next_id();
                let mut copy = self.panes[side].active().duplicate(id);
                // TC: the new tab shows the whole dir.
                copy.panel.set_filter(None);
                copy.panel.set_show(Show::All);
                self.panes[side].open_after(copy);
                return self.restore_scroll(side);
            }
            Action::CloseTab => {
                if self.panes[side].active().pinned() {
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
            Action::MoveTabLeft | Action::MoveTabRight => {
                let tabs = &mut self.panes[side];
                let (i, n) = (tabs.active_index(), tabs.items().len());
                let to = if action == Action::MoveTabLeft {
                    i.checked_sub(1)
                } else {
                    Some(i + 1).filter(|&j| j < n)
                };
                if let Some(to) = to {
                    tabs.move_to(i, to);
                }
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
            Action::SelectGroup | Action::UnselectGroup | Action::ShowMask => Some(Dialog::Mask {
                side,
                op: match action {
                    Action::SelectGroup => MaskOp::Select,
                    Action::UnselectGroup => MaskOp::Unselect,
                    _ => MaskOp::Show,
                },
                input: match panel.show() {
                    Show::Mask(m) if action == Action::ShowMask => m.clone(),
                    _ => self.masks.first().cloned().unwrap_or_else(|| "*.*".into()),
                },
            }),
            Action::Copy | Action::Move | Action::Link => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let op = match action {
                    Action::Copy => InputOp::Copy,
                    Action::Link => InputOp::Link,
                    _ => InputOp::Move,
                };
                Some(input(
                    op,
                    sources,
                    dir_input(self.panes[1 - side].active().panel.cwd()),
                ))
            }
            Action::Properties => {
                let paths = panel.targets();
                if let Some((arc, _)) = archive::split_path(panel.cwd()) {
                    return (!paths.is_empty()).then(|| self.archive_props(side, arc, paths));
                }
                (!paths.is_empty()).then(|| self.props_dialog(side, paths))
            }
            // TC: marked files get a rename mask, copied in this dir (`*.bak`, `copy_*`).
            // Not in search results: those come from many dirs, and «this dir» is only the root.
            Action::CopySame
                if panel.has_marks()
                    && panel
                        .targets()
                        .iter()
                        .all(|p| p.parent() == Some(panel.cwd())) =>
            {
                Some(input(InputOp::Copy, panel.targets(), "*.*".to_string()))
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
            // TC offers the file under the cursor (an existing one is just opened).
            Action::NewFile => {
                let name = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                Some(input(InputOp::NewFile, Vec::new(), name))
            }
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
                        icon: String::new(),
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
                        icon: String::new(),
                    })
                    .collect(),
            }),
            Action::Connect => Some(Dialog::Connect {
                side,
                url: "sftp://".into(),
                password: String::new(),
                keep: false,
                saved: self.config.connections.clone(),
                found: Vec::new(),
                browsing: None,
                at: mount::NETWORK.to_string(),
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
                // Hidden files count too, even when not shown (in an archive: its own index);
                // fall back to the listing.
                let taken = match archive::split_path(&dir) {
                    Some((a, inner)) => archive::list(&a, &inner, true).map(|v| {
                        (v.into_iter())
                            .map(|e| e.name)
                            .filter(|n| !files.iter().any(|(f, _)| f == n))
                            .collect()
                    }),
                    None => multirename::other_names(&dir, &files),
                }
                .unwrap_or_else(|_| {
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
                    help: false,
                })))
            }
            Action::Pack => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let format = Format::from_ext(&self.config.pack_format).unwrap_or(Format::Zip);
                let base = panel.cwd().to_path_buf();
                let dirs: Vec<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| base.join(&e.os_name))
                    .filter(|p| sources.contains(p))
                    .collect();
                let name = archive::archive_name(&sources, &base, format, &|p| {
                    dirs.iter().any(|d| d == p)
                });
                let path = self.panes[1 - side].active().panel.cwd().join(name);
                Some(Dialog::Pack(Box::new(dialogs::Pack {
                    side,
                    sources,
                    dirs,
                    base,
                    path: path.display().to_string(),
                    format,
                    move_after: false,
                    separate: false,
                    encrypt: false,
                    password: String::new(),
                    again: String::new(),
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
                    find_text: self.find.find_text,
                    help: false,
                    text: self.find.text.clone(),
                    case_sensitive: self.find.case_sensitive,
                    regex: self.find.regex,
                    name_regex: self.find.name_regex,
                    archives: self.find.archives,
                    min_size: self.find.min_size.clone(),
                    max_size: self.find.max_size.clone(),
                    days: self.find.days.clone(),
                    attrs: self.find.attrs.clone(),
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
                    scroll: [widget::Id::unique(), widget::Id::unique()],
                    x: 0.0,
                    offset: 0.0,
                    widths: (420.0, 420.0),
                    opts: Default::default(),
                    dirty: (false, false),
                    confirm_close: false,
                    stamps: None,
                    copying: None,
                    undo: Vec::new(),
                    editing: None,
                    edit_id: widget::Id::unique(),
                })))
            }
            Action::SyncDirs => Some(Dialog::Sync(Box::new(dialogs::SyncDlg {
                side,
                left: self.panes[0].active().target(),
                right: self.panes[1].active().target(),
                recursive: self.sync.recursive,
                content: self.sync.content,
                ignore_date: self.sync.ignore_date,
                hidden: panel.show_hidden(),
                hide: self.sync.hide.iter().copied().collect(),
                mirror: false,
                mask: self.sync.mask.clone(),
                confirm: false,
                rows: Vec::new(),
                id: 0,
                running: None,
            }))),
            Action::ChecksumCreate => {
                let sources = panel.targets();
                (!sources.is_empty()).then(|| Dialog::Checksum {
                    side,
                    sources,
                    algo: (Algo::ALL.into_iter())
                        .find(|a| a.ext() == self.config.checksum_algo)
                        .unwrap_or(Algo::Sha256),
                    separate: false,
                })
            }
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
            Dialog::Checksum {
                side,
                sources,
                algo,
                separate,
            } => {
                let t = self.panes[side].active();
                let (tab, dir, stop) = (t.id, t.panel.cwd().to_path_buf(), t.counting.clone());
                self.say(StatusKind::Busy, fl!("checksum-making"));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            // Stopped: "" (no error to show).
                            (checksum::create(&dir, &sources, algo, separate, &stop)).map_err(|e| {
                                if e.kind() == std::io::ErrorKind::Interrupted {
                                    String::new()
                                } else {
                                    crate::why::text(&e)
                                }
                            })
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |r| cosmic::Action::App(Message::Summed(tab, r)),
                )
            }
            Dialog::Tree { side, tree, .. } => match tree.current() {
                Some(n) => {
                    self.active = side;
                    self.load(side, n.path.clone(), None)
                }
                None => Task::none(),
            },
            Dialog::Props(p) => {
                // Only the touched bits, as shown now: entries keep the rest of their own modes.
                let (set, clear) = (p.mode & p.touched, !p.mode & p.touched & 0o7777);
                let (uid, gid) = (p.new_user(), p.new_group());
                if p.touched == 0 && uid.is_none() && gid.is_none() {
                    return Task::none();
                }
                let me = shagoff_core::owners::euid();
                // A new owner, a group not the user's, or files not his need root: polkit asks
                // (163), the program itself runs as root to do it.
                let as_root = me != 0
                    && (uid.is_some()
                        || p.foreign
                        || gid.is_some_and(|g| !shagoff_core::owners::my_groups().contains(&g)));
                let (side, paths, recursive) = (p.side, p.paths.clone(), p.recursive);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            use shagoff_core::props;
                            if !as_root {
                                let failed =
                                    props::apply(&paths, uid, gid, (set, clear), recursive);
                                return (failed.into_iter())
                                    .map(|(p, e)| format!("{}: {e}", p.display()))
                                    .collect();
                            }
                            let out = std::env::current_exe().and_then(|exe| {
                                std::process::Command::new("pkexec")
                                    .arg(exe)
                                    .args(props::helper_args(
                                        &paths,
                                        uid,
                                        gid,
                                        (set, clear),
                                        recursive,
                                    ))
                                    .stdin(std::process::Stdio::null())
                                    .output()
                            });
                            match out {
                                Err(e) => vec![format!("pkexec: {e}")],
                                Ok(o) if o.status.success() => Vec::new(),
                                Ok(o) => {
                                    // The helper's failures, or pkexec's refusal.
                                    let err = String::from_utf8_lossy(&o.stderr);
                                    let mut v: Vec<String> =
                                        err.lines().map(str::to_string).collect();
                                    if v.is_empty() {
                                        v.push(format!("pkexec: {}", o.status));
                                    }
                                    v
                                }
                            }
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |failed| cosmic::Action::App(Message::PropsDone(side, failed)),
                )
            }
            Dialog::Mask { side, op, input } => {
                self.masks = cmdline::remember(&self.masks, &input);
                let panel = &mut self.panes[side].active_mut().panel;
                match op {
                    MaskOp::Show => panel.set_show(Show::Mask(input)),
                    _ => panel.mark_by_mask(&Mask::parse(&input), op == MaskOp::Select),
                }
                Task::none()
            }
            Dialog::Input {
                op: InputOp::Mkdir,
                side,
                input,
                ..
            } => self.mkdir(side, input.trim()),
            Dialog::Input {
                op: InputOp::NewFile,
                side,
                input,
                ..
            } => self.new_file(side, input.trim()),
            Dialog::Input {
                op: InputOp::OpenWith,
                side,
                sources,
                input,
            } => {
                if input.trim().is_empty() {
                    return Task::none();
                }
                self.other_cmds = cmdline::remember(&self.other_cmds, &input);
                let dir = self.panes[side].active().panel.cwd().to_path_buf();
                let line = openwith::other_line(&input, &sources);
                // Watched like a toolbar command: a typo ("gimpp") is reported, not silent.
                match run_watched(&cmdline::argv(&line, &dir, None), &dir) {
                    Ok(task) => task,
                    Err(e) => {
                        self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                        Task::none()
                    }
                }
            }
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
                // In an archive: paths inside it, for the repack.
                let inner = archive::split_path(&m.dir);
                let base = inner.as_ref().map_or(m.dir.as_path(), |(_, i)| i.as_path());
                let pairs = multirename::plan(base, &rows);
                if pairs.is_empty() {
                    return Task::none();
                }
                let focus = m
                    .current
                    .as_ref()
                    .and_then(|c| rows.iter().find(|r| &r.old == c))
                    .map(|r| r.new.clone());
                if let Some((archive, _)) = inner {
                    let job = Job::Repack {
                        archive,
                        change: Change::Rename(pairs),
                    };
                    return self.start_job(m.side, OpKind::Repack, job, focus);
                }
                let job = Job::Transfer {
                    method: Method::Rename,
                    pairs,
                };
                self.start_job(m.side, OpKind::Move, job, focus)
            }
            Dialog::Pack(p) => {
                let Ok(password) = p.password() else {
                    self.dialog = Some(Dialog::Pack(p)); // Enter does nothing until they match
                    return Task::none();
                };
                let cwd = self.panes[p.side].active().target();
                let path = cwd.join(p.path.trim());
                if into_archive(&path) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
                    return Task::none();
                }
                let is_dir = |s: &Path| p.dirs.iter().any(|d| d == s);
                let groups =
                    archive::groups(&p.sources, &p.base, &path, p.format, p.separate, &is_dir);
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
                    password,
                };
                self.start_job(p.side, OpKind::Pack, job, focus)
            }
            Dialog::Connect {
                side,
                url,
                password,
                keep,
                saved,
                found,
                browsing,
                at,
                note,
            } => {
                let address = url.trim().to_string();
                // Stays open while connecting: the error lands in it, Cancel stops it.
                let busy = self.connecting.is_some();
                self.dialog = Some(Dialog::Connect {
                    side,
                    url,
                    password: password.clone(),
                    keep,
                    saved,
                    found,
                    browsing,
                    at,
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
            Dialog::Colors(c) => {
                match &self.config_handler {
                    Some(h) => {
                        let colors = self.config.set_colors(h, c.list);
                        let panel = self.config.set_panel(h, c.panel);
                        if let Err(e) = colors.and(panel) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => (self.config.colors, self.config.panel) = (c.list, c.panel),
                }
                self.scheme = shagoff_core::colors::Scheme::new(&self.config.colors)
                    .with_panel(&self.config.panel);
                Task::none()
            }
            Dialog::Toolbar(t) => {
                self.save_toolbar(t.list);
                Task::none()
            }
            Dialog::Keys(k) => {
                let list = k.result();
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_hotkeys(h, list) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.hotkeys = list,
                }
                keymap::set_own(crate::hotkeys::resolve(&self.config.hotkeys));
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
                self.updating = Some(file.clone());
                let job = Job::Repack {
                    archive,
                    change: Change::Replace { entry, file },
                };
                self.start_job(self.active, OpKind::Repack, job, None)
            }
            // Exits once the worker has stopped and cleaned its part file (finish_job).
            Dialog::ConfirmExit => {
                self.cancel_job();
                self.quitting = true;
                if self.job.is_none() {
                    return self.handle(Message::Exit);
                }
                Task::none()
            }
            Dialog::Password {
                archive,
                password,
                reply,
                open,
                ..
            } => {
                archive::set_password(&archive, Some(password));
                if let Some(r) = reply {
                    let _ = r.send(ErrorChoice::Retry);
                }
                match open {
                    Some((side, dir)) => self.load(side, dir, None),
                    None => Task::none(),
                }
            }
            // Answered with their own buttons, not Enter/OK.
            d @ (Dialog::Conflict { .. } | Dialog::Error { .. } | Dialog::DropMenu { .. }) => {
                self.dialog = Some(d);
                Task::none()
            }
        }
    }

    /// Shift+F4: create `name` in the panel's dir if it is not there, open it in the editor,
    /// put the cursor on it.
    fn new_file(&mut self, side: usize, name: &str) -> Task<Message> {
        if !ops::valid_name(name) {
            if !name.is_empty() {
                self.say(StatusKind::Error, plan_error(&PlanError::BadName));
            }
            return Task::none();
        }
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let file = cwd.join(name);
        if let Err(e) = ops::new_file(&file) {
            let err = crate::why::text(&e);
            self.say(
                StatusKind::Error,
                fl!("new-file-failed", path = name, err = err),
            );
            return Task::none();
        }
        let argv = launch::command(&self.config.editor, &["cosmic-edit"], &file);
        if let Err(err) = spawn_detached(&argv) {
            self.say(
                StatusKind::Error,
                fl!("open-failed", err = crate::why::text(&err)),
            );
        }
        self.reload(side, cwd, Some(name.to_string()))
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
            };
            return self.start_job(side, OpKind::Repack, job, first());
        }
        // In the background: on a dead network mount `mkdir` never returns.
        let (dir, name, first) = (cwd.join(name), name.to_string(), first());
        Task::perform(
            async move {
                let made = tokio::task::spawn_blocking(move || std::fs::create_dir_all(dir))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| crate::why::text(&e)));
                Message::Made {
                    side,
                    cwd,
                    name,
                    first,
                    made,
                }
            },
            cosmic::Action::App,
        )
    }

    /// Ctrl+Shift+F5: links placed by F5's rules (`link_all`, in the background); no job, no
    /// dialog. Both panes reread.
    fn make_links(&mut self, side: usize, sources: &[PathBuf], target: &Path) -> Task<Message> {
        if [sources.first().map(PathBuf::as_path), Some(target)]
            .into_iter()
            .flatten()
            .any(inside_archive)
        {
            self.say(StatusKind::Error, fl!("archive-read-only"));
            return Task::none();
        }
        // In the background: on a dead network mount a `stat` or `symlink` never returns.
        let (sources, target) = (sources.to_vec(), target.to_path_buf());
        let tab = self.panes[side].active().id;
        Task::perform(
            async move {
                let made = tokio::task::spawn_blocking(move || link_all(&sources, &target))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                Message::Linked(tab, made)
            },
            cosmic::Action::App,
        )
    }

    /// F5 / F6 / rename: resolve the target, then run the transfer on a worker thread.
    fn start_transfer(
        &mut self,
        op: InputOp,
        side: usize,
        sources: Vec<PathBuf>,
        input: &str,
    ) -> Task<Message> {
        // Search results in the panel: files from archives go out one archive dir at a time (each
        // its own job, queued), the ones on disk together.
        if op != InputOp::Rename {
            let groups = archive::split_by(&sources, |p| {
                p.parent()
                    .filter(|d| archive::within(d))
                    .map(Path::to_path_buf)
            });
            if groups.len() > 1 {
                let tasks: Vec<_> = (groups.into_iter())
                    .map(|g| self.start_transfer(op, side, g, input))
                    .collect();
                return Task::batch(tasks);
            }
        }
        // While a navigation is in flight the rows still show the dir being left; aim at where the tab is going.
        let cwd = self.panes[side].active().target();
        // From the rows' own dir: right after Enter on an archive they still show the dir left.
        let from = sources
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let from_archive = from.as_deref().and_then(archive::split_path);
        let target = cwd.join(session::expand_home(Path::new(input), &self.home));
        if op == InputOp::Link {
            return self.make_links(side, &sources, &target);
        }
        // The input's trailing `/` ("into this dir", TC) kept for `Change::Add`.
        let to_archive = (op != InputOp::Rename)
            .then(|| archive::split_path(&target))
            .flatten()
            .map(|(a, mut inner)| {
                if input.ends_with('/') && !inner.as_os_str().is_empty() {
                    inner.as_mut_os_string().push("/");
                }
                (a, inner)
            });
        match (from_archive, to_archive) {
            (Some((from, inner)), Some((to, to_inner))) => {
                let names: Vec<PathBuf> = (sources.iter())
                    .filter_map(|p| p.file_name().map(PathBuf::from))
                    .collect();
                // Inside one archive, as on disk: not onto itself, not a dir into itself (by the
                // real file: the same archive may be reached through a link).
                let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                if real(&from) == real(&to) {
                    // Into its own dir, or one file onto itself by name (`a.zip/x` → `a.zip/x`).
                    let onto_itself = matches!(&names[..], [n] if inner.join(n) == to_inner);
                    let e = if inner == to_inner || onto_itself {
                        sources.first().map(|s| PlanError::SameFile(s.clone()))
                    } else {
                        (names.iter().map(|n| inner.join(n)))
                            .find(|n| to_inner.starts_with(n))
                            .map(|n| PlanError::IntoItself(from.join(n)))
                    };
                    if let Some(e) = e {
                        self.say(StatusKind::Error, plan_error(&e));
                        return Task::none();
                    }
                }
                let job = Job::Cross {
                    from,
                    inner,
                    names,
                    to,
                    to_inner,
                    move_after: op == InputOp::Move,
                };
                return self.start_job(side, OpKind::Repack, job, None);
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
                    change: Change::Rename(vec![(inner.join(name), inner.join(input))]),
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
                    change: Change::Add {
                        sources,
                        inner,
                        move_sources: op == InputOp::Move,
                    },
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
        let replan = (op != InputOp::Rename).then_some((sources, target));
        self.start_queued(side, kind, Job::Transfer { method, pairs }, focus, replan)
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
        self.start_diff_with(None, None, None)
    }

    /// Compare the dialog's files: read them (`src` None), or the texts in hand with new options
    /// or after copying block `copy.0` (`copy.1`: left → right), or the texts `undo` (a copy undone).
    fn start_diff_with(
        &mut self,
        shown: Option<Arc<Result<shagoff_core::diff::Outcome, String>>>,
        copy: Option<(usize, bool)>,
        undo: Option<Arc<(String, String)>>,
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
                    if let Some(src) = undo {
                        return Ok(diff::Outcome::Text(diff::rows_with(
                            &src.0,
                            &src.1,
                            diff::ROWS_LIMIT,
                            o,
                        )));
                    }
                    let Some(Ok(diff::Outcome::Text(t))) = shown.as_deref() else {
                        return diff::compare_with(&a, &b, o).map_err(|e| crate::why::text(&e));
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
        let (Some(t), (top, bottom)) = (d.text(), d.view_rows()) else {
            return Task::none();
        };
        if d.blocks().is_empty() {
            return Task::none();
        }
        d.block = shagoff_core::diff::step(&t.rows, d.blocks(), d.block, delta > 0, top, bottom);
        self.diff_scroll()
    }

    fn diff_scroll(&self) -> Task<Message> {
        let Some(Dialog::Diff(d)) = &self.dialog else {
            return Task::none();
        };
        let Some(&row) = d.blocks().get(d.block) else {
            return Task::none();
        };
        // A couple of rows of context above the block; both sides, back to the line starts.
        let y = row.saturating_sub(2) as f32 * dialogs::DIFF_ROW_H;
        let to = AbsoluteOffset {
            x: Some(0.0),
            y: Some(y),
        };
        Task::batch(
            d.scroll
                .iter()
                .map(|id| scrollable::scroll_to(id.clone(), to)),
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
        // One side inside an archive: what comes out is extracted, what goes in added, each a
        // job of its own (queued).
        if let Some(ap) = shagoff_core::sync::archive_plan(&s.left, &s.right, &s.rows) {
            self.dialog = None;
            let mut tasks = Vec::new();
            if let Some((archive, inner, names, dest)) = ap.extract {
                let job = Job::Extract {
                    archive,
                    inner,
                    names,
                    dest,
                };
                tasks.push(self.start_job(side, OpKind::Extract, job, None));
            }
            for (archive, inner, sources) in ap.add {
                let change = Change::Add {
                    sources,
                    inner,
                    move_sources: false,
                };
                let job = Job::Repack { archive, change };
                tasks.push(self.start_job(side, OpKind::Repack, job, None));
            }
            if let Some((archive, entries)) = ap.delete_in {
                let job = Job::Repack {
                    archive,
                    change: Change::Delete(entries),
                };
                tasks.push(self.start_job(side, OpKind::Repack, job, None));
            }
            if !ap.delete_disk.is_empty() {
                let job = Job::Delete {
                    paths: ap.delete_disk,
                    permanent: false,
                };
                tasks.push(self.start_job(side, OpKind::Delete, job, None));
            }
            return Task::batch(tasks);
        }
        self.dialog = None;
        let job = Job::Sync {
            to_right: p.to_right,
            to_left: p.to_left,
            delete: p.delete,
            permanent: false,
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
        // The drop-downs: every group / user and the files' own (a number when not in /etc).
        let common = |ids: &[u32], all: &[(u32, String)]| {
            (ids.first())
                .filter(|g| ids.iter().all(|x| x == *g))
                .and_then(|g| all.iter().position(|(id, _)| id == g))
        };
        let file_gids: Vec<u32> = metas.iter().flatten().map(|m| m.gid()).collect();
        let mut gids = self.owners.all_gids();
        gids.extend(&file_gids);
        let groups = self.owners.groups(&gids);
        let group = common(&file_gids, &groups);
        let file_uids: Vec<u32> = metas.iter().flatten().map(|m| m.uid()).collect();
        let mut uids = self.owners.all_uids();
        uids.extend(&file_uids);
        let users = self.owners.users(&uids);
        let user = common(&file_uids, &users);
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
            gids: groups.iter().map(|(g, _)| *g).collect(),
            group_names: groups.into_iter().map(|(_, n)| n).collect(),
            group,
            group0: group,
            uids: users.iter().map(|(u, _)| *u).collect(),
            user_names: users.into_iter().map(|(_, n)| n).collect(),
            user,
            user0: user,
            foreign: {
                let me = shagoff_core::owners::euid();
                file_uids.iter().any(|u| *u != me)
            },
            usage: None,
            id: 0, // set by `start_usage`
            stop: Arc::new(AtomicBool::new(false)),
            archive: None,
        }))
    }

    /// Alt+Enter inside an archive: what its index says, nothing to change (the bits are shown
    /// greyed, no group, no Apply).
    fn archive_props(&self, side: usize, arc: PathBuf, paths: Vec<PathBuf>) -> Dialog {
        let panel = &self.panes[side].active().panel;
        let entries: Vec<&Entry> = (panel.entries().iter())
            .filter(|e| paths.contains(&panel.cwd().join(&e.os_name)))
            .collect();
        let mut facts = Vec::new();
        if let [e] = entries.as_slice() {
            facts.push((fl!("props-name"), e.name.clone()));
            let kind = if e.is_dir() {
                fl!("props-folder")
            } else if e.is_link {
                fl!("props-link-archive")
            } else {
                format::mime_type(&e.ext).unwrap_or_else(|| "—".into())
            };
            facts.push((fl!("props-type"), kind));
            facts.push((fl!("props-modified"), format::date(e.mtime, &self.tz)));
        } else {
            facts.push((fl!("props-selected"), fl!("props-items", n = paths.len())));
        }
        facts.push((fl!("props-archive"), arc.display().to_string()));
        let all = entries.iter().fold(0o7777, |a, e| a & e.mode);
        let any = entries.iter().fold(0, |a, e| a | e.mode);
        Dialog::Props(Box::new(dialogs::Props {
            side,
            paths,
            facts,
            mode: if entries.is_empty() { 0 } else { all },
            mixed: any & !all,
            own_mixed: any & !all,
            touched: 0,
            has_dir: false,
            recursive: false,
            gids: Vec::new(),
            group_names: Vec::new(),
            group: None,
            group0: None,
            uids: Vec::new(),
            user_names: Vec::new(),
            user: None,
            user0: None,
            foreign: false,
            usage: None,
            id: 0,
            stop: Arc::new(AtomicBool::new(false)),
            archive: Some(arc),
        }))
    }

    /// Count the open properties dialog's selection in the background.
    fn start_usage(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Props(p)) = &mut self.dialog else {
            return Task::none();
        };
        p.id = id;
        let (paths, stop, arc) = (p.paths.clone(), p.stop.clone(), p.archive.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match arc {
                    Some(a) => {
                        let inner: Vec<PathBuf> = (paths.iter())
                            .filter_map(|p| Some(p.strip_prefix(&a).ok()?.to_path_buf()))
                            .collect();
                        archive::usage_of(&a, &inner).ok()
                    }
                    None => shagoff_core::props::usage(&paths, &stop),
                })
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
            find_text: f.find_text,
            text: f.text.clone(),
            case_sensitive: f.case_sensitive,
            regex: f.regex,
            name_regex: f.name_regex,
            archives: f.archives,
            min_size: f.min_size.clone(),
            max_size: f.max_size.clone(),
            days: f.days.clone(),
            attrs: f.attrs.clone(),
        };
        let root = self.panes[side].active().target().join(f.dir.trim());
        let root_searched = root.clone();
        // «Искать текст» off: names only, whatever the field holds.
        let text = if f.find_text {
            f.text.trim().to_string()
        } else {
            String::new()
        };
        let q = match find_query(f, text) {
            Ok(q) => shagoff_core::search::Query {
                hidden: self.panes[side].active().panel.show_hidden(),
                ..q
            },
            Err(e) => {
                // The last search's results would read as this one's under the error.
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    if let Some(old) = f.stop.take() {
                        old.store(true, Ordering::Relaxed);
                    }
                    // A new id: what the stopped search still sends is not this one's.
                    (f.id, f.cursor, f.total) = (id, 0, 0);
                    f.results.clear();
                    f.current.clear();
                    f.root = None;
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
        self.dialog = None; // stops the search
        self.active = side;
        self.leave_locked(side);
        let t = self.panes[side].active_mut();
        t.results = Some((root.clone(), paths));
        t.branch = false;
        t.stop_branch(); // a branch read landing later would replace these
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
        // Opened right away or not at all: queued, the file would pop up out of nowhere later.
        if self.job.is_some() {
            self.say(StatusKind::Error, fl!("job-running"));
            return Task::none();
        }
        let dir = match archive::fresh_temp_dir(&archive::temp_root()) {
            Ok(d) => d,
            Err(e) => {
                self.say(
                    StatusKind::Error,
                    fl!("open-failed", err = crate::why::text(&e)),
                );
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
            How::Browse => Open::Browse(archive.join(&inner)),
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

    /// Ctrl+B / Ctrl+Shift+B (TC "branch"): every file below the dir (or the marked dirs) in one
    /// list, read in the background; again — the dir as usual.
    fn branch(&mut self, side: usize, action: Action) -> Task<Message> {
        let t = self.panes[side].active();
        if t.branch
            && let Some((root, _)) = &t.results
        {
            let root = root.clone();
            self.panes[side].active_mut().results = None;
            return self.load(side, root, None);
        }
        let cwd = t.panel.cwd().to_path_buf();
        if t.results.is_some() {
            self.say(StatusKind::Error, fl!("results-unsupported"));
            return Task::none();
        }
        let dirs: Vec<PathBuf> = if action == Action::BranchSel {
            // From the listing: inside an archive nothing is on disk.
            let dirs: std::collections::HashSet<&std::ffi::OsStr> = (t.panel.entries().iter())
                .filter(|e| e.is_dir())
                .map(|e| e.os_name.as_os_str())
                .collect();
            (t.panel.targets().into_iter())
                .filter(|p| p.file_name().is_some_and(|n| dirs.contains(n)))
                .collect()
        } else {
            vec![cwd.clone()]
        };
        if dirs.is_empty() {
            return Task::none();
        }
        let (tab, hidden) = (t.id, self.config.show_hidden);
        let id = self.next_id();
        let stop = Arc::new(AtomicBool::new(false));
        let t = self.panes[side].active_mut();
        t.branching = Some(id);
        t.branch_stop.store(true, Ordering::Relaxed); // an earlier read still going
        t.branch_stop = stop.clone();
        self.say(StatusKind::Busy, fl!("branch-reading"));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match archive::split_path(&dirs[0]) {
                    // ponytail: the index is read whole, Esc does not stop it (as entering one).
                    Some((a, _)) => Some(
                        (dirs.iter())
                            .filter_map(|d| {
                                archive::branch(&a, d.strip_prefix(&a).ok()?, hidden).ok()
                            })
                            .flatten()
                            .collect(),
                    ),
                    None => listing::branch(&dirs, hidden, &stop),
                })
                .await
                .unwrap_or(Some(Vec::new())) // the walker died: an empty branch, as before
            },
            move |paths| {
                cosmic::Action::App(Message::BranchListed {
                    tab,
                    id,
                    root: cwd.clone(),
                    paths,
                })
            },
        )
    }

    /// Esc in the viewer: the search field first, then the selection, then the viewer.
    fn lister_escape(&mut self) -> Task<Message> {
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        if l.searching {
            l.searching = false;
        } else if l.sel.is_some() {
            l.sel = None;
        } else {
            return self.handle(Message::ListerClose);
        }
        Task::none()
    }

    /// The full F3 viewer is up (not the Ctrl+Q one) and takes the keys: in place of the panels,
    /// or its own window focused (no window known to be focused: as focused, so it is never left
    /// without keys).
    fn viewer(&self) -> bool {
        self.lister.as_ref().is_some_and(|l| {
            l.quick.is_none()
                && l.window
                    .is_none_or(|w| self.core.focused_window().is_none_or(|f| f == w))
        })
    }

    /// Ctrl+Q: show what is under the active pane's cursor, if that changed. Run after every
    /// message, so any way the cursor moves (keys, mouse, quick search, a rescan) is followed.
    fn quick_follow(&mut self) -> Task<Message> {
        let side = self.active;
        let panel = &self.panes[side].active().panel;
        let entry = (panel.current()).filter(|e| e.name != PARENT);
        let key = entry.map_or_else(
            || (panel.cwd().to_path_buf(), 0, std::time::UNIX_EPOCH),
            |e| (panel.cwd().join(&e.os_name), e.size, e.mtime),
        );
        let (name, dir) = entry.map_or((String::new(), None), |e| {
            (e.name.clone(), Some(e.is_dir()))
        });
        match self.lister.as_deref() {
            // Only the Ctrl+Q one; the F3 viewer stays on its file.
            Some(l)
                if l.quick
                    .as_ref()
                    .is_some_and(|q| *q != key || l.side != side) => {}
            Some(l) if l.quick.is_some() => return Task::none(),
            // Closed (or never open): a dir count left running has no one to show it to.
            _ => {
                self.quick_stop.store(true, Ordering::Relaxed);
                return Task::none();
            }
        }
        let id = self.next_id();
        self.quick_latest.store(id, Ordering::Relaxed);
        let latest = self.quick_latest.clone();
        // A dir still being counted for the last entry: not any more.
        let stop = std::mem::take(&mut self.quick_stop);
        stop.store(true, Ordering::Relaxed);
        let stop = self.quick_stop.clone();
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        let (file, size) = (key.0.clone(), key.1);
        l.reopen(name.clone(), id);
        l.quick = Some(key);
        l.side = side;
        // Inside an archive? As F3 decides (a real dir named `x.zip` is none).
        let in_archive = archive::split_path(self.panes[side].active().panel.cwd());
        if in_archive.is_none() {
            l.file = Some(file.clone());
        }
        if dir.is_none() {
            l.set_loaded(Arc::new(Err(fl!("quick-none"))));
            return Task::none();
        }
        // The viewer reads 32 MB at most; a bigger entry would be unpacked whole for that.
        if in_archive.is_some() && size > shagoff_core::lister::LIMIT {
            let limit = (shagoff_core::lister::LIMIT >> 20).to_string();
            l.set_loaded(Arc::new(Err(fl!("quick-too-big", limit = limit))));
            return Task::none();
        }
        // An archive opens as a folder here: shown like one, what it holds (F3 shows its bytes).
        let whole = in_archive.is_none()
            && dir == Some(false)
            && Format::detect(&name).is_some_and(Format::is_tree);
        // TC: a dir shows what is in it, counted in the background.
        if dir == Some(true) || whole {
            l.set_loaded(Arc::new(Err(fl!("quick-counting"))));
            let entry = file.file_name().map(PathBuf::from).unwrap_or_default();
            // (archive, dir inside it) when counted from an archive's index.
            let indexed = match in_archive {
                _ if whole => Some((file.clone(), PathBuf::new())),
                Some((arc, inner)) => Some((arc, inner.join(entry))),
                None => None,
            };
            let key = if whole { "archive" } else { "dir" };
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || match indexed {
                        // From the archive's index: nothing is unpacked.
                        Some((arc, inner)) => {
                            // An unreadable index (archive gone, cut): say so, not a blank.
                            archive::usage(&arc, &inner).map_err(|_| ())
                        }
                        None => {
                            // A link to a dir: what it points to (`usage` does not follow links).
                            let root = std::fs::canonicalize(&file).unwrap_or(file);
                            // Stopped: dropped anyway (the cursor moved on, its id is stale).
                            shagoff_core::props::usage(&[root], &stop).ok_or(())
                        }
                    })
                    .await
                    .unwrap_or(Err(()))
                },
                move |u| {
                    let text = match u {
                        Ok(u) => {
                            let (files, dirs) = (u.files, u.dirs.saturating_sub(1));
                            let size = crate::view::human(u.bytes);
                            if key == "archive" {
                                fl!("quick-archive", files = files, dirs = dirs, size = size)
                            } else {
                                fl!("quick-dir", files = files, dirs = dirs, size = size)
                            }
                        }
                        Err(()) => fl!("quick-none"),
                    };
                    cosmic::Action::App(Message::ListerLoaded(id, Arc::new(Err(text))))
                },
            );
        }
        let read = Task::perform(
            async move {
                // Held ↓ passes many files: read only the one the cursor stops on.
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                if latest.load(Ordering::Relaxed) != id {
                    return Err(String::new()); // dropped: its id is stale
                }
                tokio::task::spawn_blocking(move || match in_archive {
                    // Inside an archive: taken out into a temp dir, read, the copy dropped.
                    Some((arc, inner)) => {
                        let tmp = archive::fresh_temp_dir(&archive::temp_root())
                            .map_err(|e| crate::why::text(&e))?;
                        let entry = file.file_name().map(PathBuf::from).unwrap_or_default();
                        let r = match archive::extract_one(&arc, &inner, &entry, &tmp, &stop) {
                            Some(f) => lister::Loaded::read(&f, &name),
                            None => Err(fl!("quick-none")),
                        };
                        let _ = std::fs::remove_dir_all(&tmp);
                        r
                    }
                    None => lister::Loaded::read(&file, &name),
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerLoaded(id, Arc::new(r))),
        );
        Task::batch([read, lister_scroll(l, 0.0, 0.0)])
    }

    /// F3 with the built-in viewer: the file under the cursor (extracted first inside an archive).
    fn view_current(&mut self, side: usize) -> Task<Message> {
        let panel = &self.panes[side].active().panel;
        let cwd = panel.cwd().to_path_buf();
        let Some(e) = panel.current().filter(|e| !e.is_dir() && e.name != PARENT) else {
            return Task::none();
        };
        let (os_name, name) = (e.os_name.clone(), e.name.clone());
        let (dir, own) = located(&cwd, &os_name);
        if let Some((arc, inner)) = archive::split_path(&dir) {
            return self.open_from_archive(side, arc, inner, own, How::Lister);
        }
        let file = cwd.join(&os_name);
        if let Some(why) = unopenable(&file, &name) {
            self.say(StatusKind::Error, why);
            return Task::none();
        }
        self.open_lister(side, file, name)
    }

    fn open_lister(&mut self, side: usize, file: PathBuf, name: String) -> Task<Message> {
        let id = self.next_id();
        let mut window = Task::none();
        // Ctrl+Q's viewer: the full one replaces it (in its own window, if so set).
        if self.lister.as_ref().is_some_and(|l| l.quick.is_some()) {
            self.lister = None;
        }
        match &mut self.lister {
            Some(l) => {
                l.reopen(name.clone(), id);
                l.side = side; // N / P walk the pane F3 was pressed in
                if let Some(w) = l.window {
                    window = Task::batch([
                        cosmic::ApplicationExt::set_window_title(self, name.clone(), w),
                        cosmic::iced::window::gain_focus(w),
                    ]);
                }
            }
            None => {
                let mut l = Lister::new(side, name.clone(), id, self.config.lister_size());
                // TC: the Lister is a window of its own.
                if self.config.lister_window {
                    let (w, open) = cosmic::iced::window::open(cosmic::iced::window::Settings {
                        size: cosmic::iced::Size::new(900.0, 650.0),
                        min_size: Some(cosmic::iced::Size::new(360.0, 240.0)),
                        decorations: false,
                        exit_on_close_request: false,
                        ..Default::default()
                    });
                    l.window = Some(w);
                    window = Task::batch([
                        open.discard(),
                        cosmic::ApplicationExt::set_window_title(self, name.clone(), w),
                    ]);
                }
                self.lister = Some(Box::new(l));
            }
        }
        self.search = None;
        if let Some(l) = self.lister.as_deref_mut() {
            l.file = Some(file.clone());
        }
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
        Task::batch([read, top, window])
    }

    /// Keys while the viewer is open; `None`: not the viewer's (help, settings…), act as usual.
    fn lister_action(&mut self, action: Action) -> Option<Task<Message>> {
        if matches!(action, Action::Left | Action::Right) {
            self.lister.as_ref()?;
            return Some(self.lister_scroll_x(action == Action::Right));
        }
        let l = self.lister.as_deref_mut()?;
        Some(match action {
            Action::Help | Action::About | Action::Settings | Action::Donate | Action::F12 => {
                return None;
            }
            // Num+ / Num-: bigger / smaller text, as + / - (TC zooms with them).
            Action::SelectGroup => return Some(self.lister_zoom(true)),
            Action::UnselectGroup => return Some(self.lister_zoom(false)),
            Action::View => self.lister_find(true, true),
            Action::Mkdir => self.lister_search(),
            Action::SelectAll => {
                l.select_all();
                Task::none()
            }
            Action::ClipCopy => match l.selected_text() {
                Some(t) => {
                    let n = t.chars().count();
                    self.say(StatusKind::Info, fl!("lister-copied", n = n));
                    cosmic::iced::clipboard::write(t)
                }
                None => Task::none(),
            },
            // Home / End of a big file: its first / last window.
            Action::Home if l.window_for(Slide::Home).is_some() => self.lister_slide(Slide::Home),
            Action::End if l.window_for(Slide::End).is_some() => self.lister_slide(Slide::End),
            a => match l.scroll_y(a) {
                Some(y) => {
                    let scroll = lister_scroll(l, l.offset.0, y);
                    match l.edge() {
                        Some(s) => Task::batch([scroll, self.lister_slide(s)]),
                        None => scroll,
                    }
                }
                None => Task::none(),
            },
        })
    }

    /// Read another window of the viewer's big file (`Message::ListerPart`).
    fn lister_slide(&mut self, slide: Slide) -> Task<Message> {
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        let (Some(w), Some(file)) = (l.window_for(slide), l.file.clone()) else {
            return Task::none();
        };
        // Home / End may come while another is read: its reply is dropped (a search's too).
        let searching = matches!(l.slide, Some(Slide::Find(_)));
        l.slide = Some(slide);
        let id = l.id;
        if searching {
            self.clear_busy();
        }
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lister::Loaded::window(&file, w))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerPart(id, slide, Arc::new(r))),
        )
    }

    /// The viewer's text one size up or down: the same setting as in Settings, saved.
    fn lister_zoom(&mut self, up: bool) -> Task<Message> {
        // A picture: its zoom (TC), never the text size; still reading: not known which yet.
        if let Some(l) = self.lister.as_deref_mut()
            && (l.loaded.is_none() || l.mode == shagoff_core::lister::Mode::Image)
        {
            return match l.zoom_image(up) {
                Some((x, y)) => lister_scroll(l, x, y),
                None => Task::none(),
            };
        }
        let size = config::size_step(self.config.lister_size(), up);
        if size == self.config.lister_size() {
            return Task::none();
        }
        self.set(crate::drawer::Setting::ListerSize(size))
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
            // Not in this window of a big file: on through the next ones.
            None if let Some(s) = l.search_on(forward) => {
                let q = l.query.clone();
                self.say(StatusKind::Busy, fl!("lister-searching", query = q));
                self.lister_slide(s)
            }
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

    /// `action` would change an archive opened from inside another (155): a temp copy.
    fn nested_change(&self, side: usize, action: Action) -> bool {
        let nested = |s: usize| {
            let t = self.panes[s].active().target();
            t.starts_with(archive::temp_root()) && archive::split_path(&t).is_some()
        };
        use Action as A;
        let here = matches!(
            action,
            A::Delete
                | A::DeletePermanent
                | A::Mkdir
                | A::Rename
                | A::NewFile
                | A::MultiRename
                | A::Edit
                | A::Move
                | A::CopySame
                | A::Properties
                | A::ClipCut
                | A::ClipPaste
        );
        (here && nested(side)) || (matches!(action, A::Copy | A::Move) && nested(1 - side))
    }

    /// Actions that would change an archive (this panel inside one, or the other one as target).
    fn read_only(&self, side: usize, action: Action) -> bool {
        let inside = |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
        // F4, F7, F8, Shift+F6 inside and F5 / F6 into or out of one rewrite the archive.
        match action {
            // Shift+F5 inside one copies from the archive into itself.
            Action::CopySame | Action::NewFile => inside(side),
            // A link to or into an archive's inside would point at nothing on disk.
            Action::Pack | Action::Unpack | Action::Link => inside(side) || inside(1 - side),
            Action::ChecksumCreate => inside(side),
            // One side in an archive syncs (158): jobs extract from it and add to it.
            Action::SyncDirs => inside(side) && inside(1 - side),
            _ => false,
        }
    }

    fn start_delete(&mut self, side: usize, permanent: bool, paths: Vec<PathBuf>) -> Task<Message> {
        // Search results in the panel: each archive rewritten once, the files on disk together.
        let groups = archive::split_by(&paths, |p| {
            p.parent().and_then(archive::split_path).map(|(a, _)| a)
        });
        if groups.len() > 1 {
            let tasks: Vec<_> = (groups.into_iter())
                .map(|g| self.start_delete(side, permanent, g))
                .collect();
            return Task::batch(tasks);
        }
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
        self.start_queued(side, kind, job, focus, None)
    }

    fn start_queued(
        &mut self,
        side: usize,
        kind: OpKind,
        job: Job,
        focus: Option<String>,
        replan: Option<(Vec<PathBuf>, PathBuf)>,
    ) -> Task<Message> {
        if changes_nested(&job) {
            self.say(StatusKind::Error, fl!("nested-read-only"));
            return Task::none();
        }
        // One at a time: a second one waits in the queue for the first to end (TC F2 «В очередь»).
        if self.job.is_some() {
            self.queue.push_back(Queued {
                side,
                kind,
                job,
                focus,
                replan,
            });
            self.say(StatusKind::Info, fl!("job-queued", n = self.queue.len()));
            return Task::none();
        }
        let (control, events) = jobs::spawn(job);
        self.job = Some(Running {
            side,
            kind,
            done: 0,
            total: 0,
            current: String::new(),
            control,
            focus,
            open: None,
            hidden: false,
        });
        Task::run(events, |e| cosmic::Action::App(Message::Op(e)))
    }

    /// Cancel stops what waits too: a queued delete must not start on its own right after.
    fn cancel_job(&mut self) {
        if let Some(j) = &self.job {
            j.control.cancel.store(true, Ordering::Relaxed);
        }
        self.queue.clear();
        self.held = None; // dropping the reply answers Cancel
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
                self.ask(Dialog::Conflict {
                    src,
                    dst,
                    reply,
                    name,
                });
            }
            jobs::Event::Error { path, error, reply } if archive::needs_password(&error) => {
                let wrong = archive::password(&path).is_some();
                self.ask(Dialog::Password {
                    archive: path,
                    password: String::new(),
                    wrong,
                    reply: Some(reply),
                    open: None,
                });
                return widget::text_input::focus(self.input_id.clone());
            }
            jobs::Event::Error { path, error, reply } => {
                self.ask(Dialog::Error { path, error, reply });
            }
            jobs::Event::Finished(report) => return self.finish_job(&report),
        }
        Task::none()
    }

    /// The job waits for the answer: an open dialog is not replaced, the question comes after it.
    fn ask(&mut self, d: Dialog) {
        if self.dialog.is_some() {
            self.held = Some(d);
            self.say(StatusKind::Info, fl!("job-waits-answer"));
        } else {
            self.dialog = Some(d);
        }
    }

    /// Unmark what was processed and reload both panes (cursor on the renamed entry, if any).
    fn finish_job(&mut self, report: &Report) -> Task<Message> {
        let Some(job) = self.job.take() else {
            return Task::none();
        };
        // Not put back (an error, cancelled): the next tick asks about that change again.
        if let Some(file) = self.updating.take()
            && (report.cancelled || report.completed.is_empty())
            && let Some(e) = self.edited.iter_mut().find(|e| e.file == file)
        {
            e.mtime = SystemTime::UNIX_EPOCH;
        }
        if self.quitting {
            return self.handle(Message::Exit);
        }
        let side = job.side;
        let mut view = Task::none();
        let mut browse = None;
        let mut focus = job.focus;
        // N / P in the viewer inside an archive, its extraction cancelled or failed: the cursor
        // goes back to the file still on screen.
        if let Some((Open::Lister(_), file)) = &job.open
            && (report.cancelled || !file.exists())
            && let Some(name) = (self.lister.as_ref())
                .filter(|l| l.side == side)
                .map(|l| l.name.clone())
        {
            let panel = &mut self.panes[side].active_mut().panel;
            if let Some(i) = panel.entries().iter().position(|e| e.name == name) {
                panel.set_cursor(i);
            }
            focus = Some(name);
        }
        if let Some((open, file)) = job.open
            && !report.cancelled
            && file.exists()
        {
            match open {
                Open::Run(argv) => {
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(
                            StatusKind::Error,
                            fl!("open-failed", err = crate::why::text(&err)),
                        );
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
                    Err(err) => self.say(
                        StatusKind::Error,
                        fl!("open-failed", err = crate::why::text(&err)),
                    ),
                },
                Open::Lister(name) => view = self.open_lister(side, file, name),
                Open::Browse(back) => {
                    self.nested.insert(file.clone(), back);
                    browse = Some(file);
                }
            }
        }
        self.panes[side]
            .active_mut()
            .panel
            .unmark(&report.completed);
        // A tab still loading a dir is left to that read (reading it again would drop the cursor
        // Backspace asked for); the others are read again.
        let again = |app: &mut Self, s: usize, focus: Option<String>| {
            let t = app.panes[s].active();
            match t.pending {
                Some(_) => Task::none(),
                None => {
                    let cwd = t.panel.cwd().to_path_buf();
                    app.reload(s, cwd, focus)
                }
            }
        };
        let reload = Task::batch([
            match browse {
                Some(arc) => self.load(side, arc, None),
                None => again(self, side, focus),
            },
            again(self, 1 - side, None),
            view,
        ]);
        // The next one in the background, as TC's queue.
        while let Some(mut q) = self.queue.pop_front() {
            if let (Some((sources, target)), Job::Transfer { pairs, .. }) = (&q.replan, &mut q.job)
            {
                match ops::plan(sources, target) {
                    Ok(p) => *pairs = p,
                    Err(e) => {
                        // Under the error already shown (the job just ended, one dropped before).
                        let text = match self.status.take() {
                            Some(s) if s.kind == StatusKind::Error => {
                                format!("{}\n{}", s.text, plan_error(&e))
                            }
                            _ => plan_error(&e),
                        };
                        self.say(StatusKind::Error, text);
                        continue;
                    }
                }
            }
            let next = self.start_job(q.side, q.kind, q.job, q.focus);
            if let Some(j) = &mut self.job {
                j.hidden = true;
            }
            return Task::batch([reload, next]);
        }
        reload
    }

    /// Thumbnails view: start making the pictures on screen that are not made yet, a few at a
    /// time (each one done asks for the next through `update`).
    fn want_thumbs(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let cell_h = crate::thumbs::cell_h(self.row_h());
        let covered = self.viewer_shown();
        for side in 0..2 {
            let t = self.panes[side].active();
            let hidden = covered.is_some_and(|q| q.is_none_or(|s| s == side));
            if !t.thumbs || hidden || archive::within(t.panel.cwd()) {
                continue;
            }
            let entries = t.panel.entries();
            let range =
                viewport::thumb_range(entries.len(), t.thumb_cols(), cell_h, t.offset, t.height);
            for e in &entries[range] {
                if self.thumbs_busy >= crate::thumbs::BUSY_MAX {
                    break;
                }
                if e.is_dir() {
                    continue;
                }
                let path = t.panel.cwd().join(&e.os_name);
                if self.thumbs.get(&path).is_some_and(|(m, _)| *m == e.mtime)
                    || !crate::thumbs::wanted(&e.name, e.size)
                {
                    continue;
                }
                // ponytail: the whole cache goes at once; an LRU if big picture dirs thrash it.
                if self.thumbs.len() >= crate::thumbs::CACHE_MAX {
                    self.thumbs.clear();
                }
                self.thumbs.insert(path.clone(), (e.mtime, None));
                self.thumbs_busy += 1;
                let mtime = e.mtime;
                let p = path.clone();
                tasks.push(Task::perform(
                    async move {
                        (tokio::task::spawn_blocking(move || crate::thumbs::make(&p)).await)
                            .ok()
                            .flatten()
                    },
                    move |pic| cosmic::Action::App(Message::Thumb(path, mtime, pic)),
                ));
            }
        }
        Task::batch(tasks)
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
        if t.brief && t.tree.is_none() {
            let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
            let len = t.panel.entries().len();
            t.col = viewport::brief_first_col(len, t.panel.cursor(), rows, cols, t.col);
            return Task::none();
        }
        let (row, row_h) = match (&t.tree, t.thumbs) {
            (Some(pt), _) => (pt.tree.cursor, row_h),
            (None, true) => (
                t.panel.cursor() / t.thumb_cols(),
                crate::thumbs::cell_h(row_h),
            ),
            (None, false) => (t.panel.cursor(), row_h),
        };
        match viewport::scroll_to_cursor(row, row_h, t.offset, t.height) {
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
        // A new look at About asks again: an old answer may no longer hold.
        if d == Some(Drawer::About) {
            self.update_check = None;
        }
        self.drawer = d;
    }

    /// F1 / About / Ctrl+,: open that drawer, or close it if it is the one open.
    fn toggle_drawer(&mut self, action: Action) {
        let open = match action {
            Action::Help => Some(Drawer::Help),
            Action::About => Some(Drawer::About),
            Action::Donate => Some(Drawer::Donate),
            _ => None,
        };
        let same = match (&self.drawer, &open) {
            (Some(cur), Some(d)) => std::mem::discriminant(cur) == std::mem::discriminant(d),
            (Some(cur), None) => matches!(cur, Drawer::Settings(_)),
            (None, _) => false,
        };
        if same {
            return self.set_drawer(None);
        }
        // Settings: the form (and the installed fonts) only when they open.
        let d = open.unwrap_or_else(|| {
            Drawer::Settings(SettingsForm {
                viewer: self.config.viewer.join(" "),
                editor: self.config.editor.join(" "),
                home: self
                    .config
                    .home_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                fonts: drawer::font_families(),
                font_filter: String::new(),
            })
        });
        self.set_drawer(Some(d));
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
            Setting::TerminalApp(id) => c.terminal_app = id,
            Setting::TermBuiltin(b) => c.term_builtin = b,
            Setting::TermShell(s) => c.term_shell = s,
            Setting::TermRows(n) => c.term_rows = n,
            Setting::Theme(i) => c.app_theme = config::AppTheme::ALL[i.min(2)],
            Setting::Skin(i) => c.skin = config::Skin::ALL[i.min(1)],
            Setting::ShowFkeys(b) => c.show_fkeys = b,
            Setting::DividerLine(b) => c.divider_line = b,
            Setting::HumanSizes(b) => c.human_sizes = Some(b),
            Setting::ActiveFrame(b) => c.active_frame = b,
            Setting::ShowCmdline(b) => c.show_cmdline = b,
            Setting::KeepHistory(b) => c.keep_history = b,
            Setting::InternalViewer(b) => c.internal_viewer = b,
            Setting::ShowHidden(b) => c.show_hidden = b,
            Setting::RightMarks(b) => c.right_marks = b,
            Setting::ListerWindow(b) => c.lister_window = b,
            // 0: own program (typed below), else `builtins()[i - 1]`.
            Setting::F12(0) => {
                if crate::toolbar::builtin(&c.f12.cmd).is_some() {
                    c.f12 = ToolButton::default();
                }
            }
            Setting::F12(i) => {
                if let Some(b) = crate::toolbar::builtins().get(i - 1) {
                    c.f12 = ToolButton {
                        cmd: b.cm.into(),
                        ..ToolButton::default()
                    };
                }
            }
            Setting::F12Cmd(s) => c.f12.cmd = s,
            Setting::F12Tip(s) => c.f12.tip = s,
            Setting::F12Params(s) => c.f12.params = s,
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
            Setting::PanelFont(i) => {
                c.panel_font = match &self.drawer {
                    Some(Drawer::Settings(f)) => i
                        .checked_sub(1)
                        .and_then(|i| f.fonts.get(i))
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            }
            Setting::PanelSize(s) => c.panel_size = s,
            Setting::ListerSize(s) => c.lister_size = s,
            Setting::PackFormat(i) => {
                if let Some(f) = Format::PACK.get(i) {
                    c.pack_format = f.ext().into();
                }
            }
        }
        if let Some(h) = &self.config_handler
            && let Err(e) = c.write_entry(h)
        {
            self.config_not_saved(&e);
        }
        self.apply_config(c)
    }

    /// New settings, from the settings drawer or the config watcher (a hand edit).
    fn apply_config(&mut self, c: Config) -> Task<Message> {
        let old = std::mem::replace(&mut self.config, c);
        if old.hotkeys != self.config.hotkeys {
            keymap::set_own(crate::hotkeys::resolve(&self.config.hotkeys));
        }
        self.scheme =
            shagoff_core::colors::Scheme::new(&self.config.colors).with_panel(&self.config.panel);
        self.cols = self.config.columns;
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
        // Off by any path (the menu, a hand edit): the shell ends with its terminal.
        if !self.config.term_builtin {
            (self.term, self.term_tx, self.term_full, self.term_focused) =
                (None, None, false, false);
        }
        // A new text size or theme: the running terminal too.
        if old.panel_size() != self.config.panel_size() || old.app_theme != self.config.app_theme {
            self.term_restyle();
        }
        if old.show_hidden != self.config.show_hidden {
            tasks.push(self.apply_hidden());
        }
        if old.panel_font != self.config.panel_font {
            self.face = font_named(&self.config.panel_font);
        }
        // Changed under the open Ctrl+D (by hand, another window): its rows are indices into the
        // list, so they are rebuilt, the cursor kept on the same entry.
        if old.hotlist != self.config.hotlist
            && let Some(Dialog::List {
                kind: ListKind::Hotlist,
                items,
                cursor,
                ..
            }) = &self.dialog
        {
            let was = items
                .get(*cursor)
                .map(|it| (it.label.clone(), it.path.clone()));
            let at = *cursor;
            self.hotlist_refresh(at);
            if let (Some(was), Some(Dialog::List { items, cursor, .. })) = (was, &mut self.dialog)
                && let Some(i) = items
                    .iter()
                    .position(|it| (&it.label, &it.path) == (&was.0, &was.1))
            {
                *cursor = i;
            }
        }
        if let Some(l) = &mut self.lister
            && l.size != self.config.lister_size()
        {
            // The same top row at the new row height, the wrapped lines rewrapped.
            let row = l.offset.1 / l.row_h();
            l.set_size(self.config.lister_size());
            let (x, y) = (l.offset.0, row * l.row_h());
            tasks.push(lister_scroll(l, x, y));
        }
        if old.row_h() != self.config.row_h() {
            // Offsets are in pixels of the old row height: bring each cursor back into view.
            let scale = self.config.row_h() / old.row_h();
            let cells =
                crate::thumbs::cell_h(self.config.row_h()) / crate::thumbs::cell_h(old.row_h());
            for side in 0..2 {
                // Hidden tabs too: they are shown with their stored offset.
                for t in self.panes[side].items_mut() {
                    t.offset *= if t.thumbs { cells } else { scale };
                }
                let tab = self.panes[side].active().id;
                tasks.push(self.reveal(side, tab));
                tasks.push(self.restore_scroll(side));
            }
        }
        Task::batch(tasks)
    }

    /// Mounts change rarely and procfs never blocks: re-read with every listing.
    /// gvfs network mounts are dirs under `$XDG_RUNTIME_DIR/gvfs`, not separate mounts: that dir
    /// blocks while one of them hangs, so it is read in the background (`read_gvfs`).
    fn refresh_mounts(&mut self) {
        self.gvfs_want = true;
        self.set_drives();
    }

    fn set_drives(&mut self) {
        if let Ok(m) = std::fs::read_to_string("/proc/self/mounts") {
            self.drives = drives::parse(&m, &self.home);
        }
        self.drives.extend(self.gvfs.iter().cloned());
    }

    /// Run after every `update`: the gvfs dir read asked for by `refresh_mounts`, if none is
    /// under way (a hung one would otherwise pile up threads).
    fn read_gvfs(&mut self) -> Task<Message> {
        if !self.gvfs_want || self.gvfs_busy {
            return Task::none();
        }
        let Some(root) = mount::gvfs_root() else {
            return Task::none();
        };
        (self.gvfs_want, self.gvfs_busy) = (false, true);
        Task::perform(
            async move {
                let drives = tokio::task::spawn_blocking(move || mount::gvfs_drives(&root));
                Message::Gvfs(drives.await.unwrap_or_default())
            },
            cosmic::Action::App,
        )
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

    /// Context menu "Open with…": the programs for the file under the cursor, found off the UI
    /// thread (`gio` reads the file).
    fn open_with(&mut self, side: usize) -> Task<Message> {
        let panel = &self.panes[side].active().panel;
        let Some(file) = (panel.current())
            .filter(|e| !e.is_dir() && e.name != PARENT)
            .map(|e| panel.cwd().join(&e.os_name))
        else {
            return Task::none();
        };
        if inside_archive(&file) {
            self.say(StatusKind::Error, fl!("open-with-archive"));
            return Task::none();
        }
        use i18n_embed::LanguageLoader;
        let files = panel.targets();
        let lang = crate::i18n::LANGUAGE_LOADER
            .current_language()
            .language
            .to_string();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || openwith::apps(&file, &lang))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::OpenWithApps(side, files, r)),
        )
    }

    /// Enter / click on entry `i` of the open list.
    fn pick(&mut self, i: usize) -> Task<Message> {
        // Into a submenu or back out of it: the menu stays open.
        if let Some(Dialog::List { items, .. }) = &self.dialog {
            match items.get(i).map(|it| it.kind) {
                Some(Item::Sub(s)) => {
                    self.hot_sub.push(s);
                    self.hotlist_refresh(1); // past "‹ back"
                    return Task::none();
                }
                Some(Item::Back) => {
                    self.hotlist_back();
                    return Task::none();
                }
                Some(Item::All) => {
                    use i18n_embed::LanguageLoader;
                    let lang = crate::i18n::LANGUAGE_LOADER
                        .current_language()
                        .language
                        .to_string();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                openwith::all_apps(&openwith::data_dirs(), &lang)
                            })
                            .await
                            .unwrap_or_default()
                        },
                        |apps| cosmic::Action::App(Message::OpenWithAll(apps)),
                    );
                }
                _ => {}
            }
        }
        let Some(Dialog::List {
            kind, side, items, ..
        }) = self.dialog.take()
        else {
            return Task::none();
        };
        if kind == ListKind::OpenWith {
            let files = std::mem::take(&mut self.open_with);
            if items.get(i).is_some_and(|it| it.kind == Item::Other) {
                self.dialog = Some(Dialog::Input {
                    op: InputOp::OpenWith,
                    side,
                    sources: files,
                    input: self.other_cmds.first().cloned().unwrap_or_default(),
                });
                return widget::text_input::focus(self.input_id.clone());
            }
            if let Some(item) = items.get(i)
                && let Err(e) = spawn_detached(&openwith::launch_argv(&item.path, &files))
            {
                self.say(
                    StatusKind::Error,
                    fl!("open-failed", err = crate::why::text(&e)),
                );
            }
            return Task::none();
        }
        if kind == ListKind::Context {
            use cosmic::widget::menu::Action as _;
            return match items.get(i).map(|it| it.kind) {
                Some(Item::Act(act)) => self.handle(act.message()),
                _ => Task::none(),
            };
        }
        if kind == ListKind::Shells {
            let path = items
                .get(i)
                .map(|it| it.path.to_string_lossy().into_owned());
            let Some(path) = path.filter(|p| *p != self.config.term_shell) else {
                return Task::none();
            };
            // A new shell from now on: the running one ends with its terminal.
            self.term = None;
            let task = self.set(Setting::TermShell(path));
            self.term_start();
            return task;
        }
        if kind == ListKind::Terminals {
            let id = items
                .get(i)
                .map(|it| it.path.to_string_lossy().into_owned());
            return id.map_or(Task::none(), |id| self.set(Setting::TerminalApp(id)));
        }
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
            // The config is text (RON): a path that is not UTF-8 can't be written to it.
            if cwd.to_str().is_none() {
                self.dialog = None;
                self.say(StatusKind::Error, fl!("hotlist-not-utf8"));
                return Task::none();
            }
            if !self.config.hotlist.iter().any(|e| e.path == cwd) {
                let mut list = self.config.hotlist.clone();
                // Into the submenu shown, before its `--`.
                let at = (self.hot_sub.last()).map_or(list.len(), |&s| hotlist::sub_end(&list, s));
                list.insert(
                    at,
                    HotEntry {
                        name: hotlist::menu_name(&cwd),
                        path: cwd,
                    },
                );
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

    /// Hotlist rows as TC's menu, for the submenu open (`hot_sub`): "‹ back" in a submenu, its
    /// favourites and submenus, a line, "add current dir", "configure…".
    fn hotlist_items(&self) -> Vec<ListItem> {
        let own = |label, kind| ListItem {
            label,
            path: PathBuf::new(),
            kind,
            icon: String::new(),
        };
        let list = &self.config.hotlist;
        // A stale index (the config changed under the open menu): back to the top.
        let open = self
            .hot_sub
            .last()
            .copied()
            .filter(|&s| list.get(s).is_some_and(hotlist::is_sub));
        let mut items: Vec<ListItem> = Vec::new();
        if let Some(s) = open {
            items.push(own(
                // No hot letter of its own: the submenu's would take it from the items inside.
                format!("‹ {}", hotkey_free(hotlist::sub_name(&list[s]))),
                Item::Back,
            ));
        }
        items.extend(hotlist::children(list, open).into_iter().map(|i| {
            let e = &list[i];
            match () {
                _ if hotlist::is_sep(e) => own(String::new(), Item::Sep),
                _ if hotlist::is_sub(e) => own(hotlist::sub_name(e).into(), Item::Sub(i)),
                _ => ListItem {
                    label: e.name.clone(),
                    path: e.path.clone(),
                    kind: Item::Hot(i),
                    icon: String::new(),
                },
            }
        }));
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
                    self.config_not_saved(&e);
                }
            }
            None => self.config.pack_format = v,
        }
    }

    /// The widths on screen into the config, if they changed.
    fn save_columns(&mut self) {
        if self.cols == self.config.columns {
            return;
        }
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_columns(h, self.cols) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.columns = self.cols,
        }
    }

    /// The held toolbar button let go: the bar saved if it moved.
    fn tool_drop(&mut self) {
        if self.tool_drag.release() {
            // Moved in place: taken out, or the setter would see no change to write.
            let list = std::mem::take(&mut self.config.toolbar);
            self.save_toolbar(list);
        }
    }

    fn save_toolbar(&mut self, list: Vec<ToolButton>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_toolbar(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.toolbar = list,
        }
    }

    /// A toolbar button: its internal command, or its program with the parameters filled in.
    fn run_tool(&mut self, i: usize) -> Task<Message> {
        match self.config.toolbar.get(i) {
            Some(b) => self.run_button(b.clone()),
            None => Task::none(),
        }
    }

    /// A toolbar button or F12: its internal command, or its program with the parameters expanded.
    fn run_button(&mut self, b: ToolButton) -> Task<Message> {
        if let Some(c) = crate::toolbar::builtin(&b.cmd) {
            return self.update(Message::Key(c.action));
        }
        if b.cmd.trim().is_empty() || b.cmd == crate::toolbar::SEP {
            return Task::none();
        }
        let (panel, other) = (
            &self.panes[self.active].active().panel,
            &self.panes[1 - self.active].active().panel,
        );
        let cwd = panel.cwd().to_path_buf();
        let selected: Vec<String> = (panel.targets().iter())
            .map(|p| p.strip_prefix(&cwd).unwrap_or(p).display().to_string())
            .collect();
        let vars = cmdline::Vars {
            dir: &cwd,
            other: other.cwd(),
            name: panel
                .current()
                .map(|e| e.name.as_str())
                .filter(|&n| n != ".."),
            selected: &selected,
        };
        let line = cmdline::append(&b.cmd, &cmdline::expand(&b.params, &vars));
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let dir = cmdline::start_dir(&b.dir, &vars, &home);
        match run_watched(&cmdline::argv(&line, &dir, None), &dir) {
            Ok(task) => task,
            Err(e) => {
                self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                Task::none()
            }
        }
    }

    /// A setting could not be written: it holds until exit only — said, not just logged.
    fn config_not_saved(&mut self, e: &dyn std::fmt::Display) {
        log::warn!("config: {e}");
        self.say(
            StatusKind::Error,
            fl!("config-not-saved", err = e.to_string()),
        );
    }

    fn save_hotlist(&mut self, list: Vec<HotEntry>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_hotlist(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.hotlist = list,
        }
    }

    fn save_connections(&mut self, list: Vec<String>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_connections(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.connections = list,
        }
    }

    /// Rebuild the open hotlist menu (after a submenu change or an edit), cursor at `cursor`
    /// or the next row that is not a line.
    fn hotlist_refresh(&mut self, cursor: usize) {
        let fresh = self.hotlist_items();
        if let Some(Dialog::List {
            items, cursor: c, ..
        }) = &mut self.dialog
        {
            let i = cursor.min(fresh.len() - 1);
            *c = (i..fresh.len())
                .find(|&j| fresh[j].kind != Item::Sep)
                .unwrap_or(i);
            *items = fresh;
        }
    }

    /// Out of the open submenu, the cursor on it (as a TC menu).
    fn hotlist_back(&mut self) {
        let left = self.hot_sub.pop();
        self.hotlist_refresh(0);
        if let Some(Dialog::List { items, cursor, .. }) = &mut self.dialog
            && let Some(i) = items
                .iter()
                .position(|it| Some(it.kind) == left.map(Item::Sub))
        {
            *cursor = i;
        }
    }

    /// Delete on hotlist entry `i`: drop it, save, refresh the open list in place.
    fn hotlist_remove(&mut self, i: usize) -> Task<Message> {
        let mut list = self.config.hotlist.clone();
        if i < list.len() {
            list.remove(i);
            self.save_hotlist(list);
        }
        let at = match &self.dialog {
            Some(Dialog::List { cursor, .. }) => *cursor,
            _ => 0,
        };
        self.hotlist_refresh(at);
        Task::none()
    }

    /// The command line takes input: shown, and nothing modal or in front of the panels.
    fn cmd_ready(&self) -> bool {
        self.config.show_cmdline
            && self.dialog.is_none()
            && self.drawer.is_none()
            && !self.viewer()
            && self.search.is_none()
            && !self.busy()
    }

    /// Num+ / Num− and "Open with command": ↑ older entry, ↓ newer (TC's drop-down history).
    fn mask_history(&mut self, action: Action) -> Option<Task<Message>> {
        let (list, input) = match &mut self.dialog {
            Some(Dialog::Mask { input, .. }) => (&self.masks, input),
            Some(Dialog::Input {
                op: InputOp::OpenWith,
                input,
                ..
            }) => (&self.other_cmds, input),
            _ => return None,
        };
        let step = match action {
            Action::Up => cmdline::previous(list, input),
            Action::Down => cmdline::next(list, input),
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
        // The names in `dir` (relative: under the panel's dir), for a name not in UTF-8
        // (`cmdline::word`). Read, not the panel's list: a filter hides some, search results hold
        // paths. Only for such names, so rarely.
        let siblings = |dir: &Path| -> Option<Vec<OsString>> {
            let rd = std::fs::read_dir(panel.cwd().join(dir)).ok()?;
            Some(rd.flatten().map(|e| e.file_name()).collect())
        };
        let word = match action {
            Action::Mark if !self.cmdline.is_empty() => {
                self.cmdline.push(' ');
                None
            }
            Action::CmdName | Action::CmdPath | Action::CmdCwd => {
                let path = match action {
                    Action::CmdCwd => panel.cwd().to_path_buf(),
                    _ => {
                        let e = panel.current().filter(|e| e.name != PARENT)?;
                        match action {
                            Action::CmdPath => panel.cwd().join(&e.os_name),
                            // Search results / branch: the path below the root, as shown.
                            _ => Path::new(&e.os_name)
                                .strip_prefix(panel.cwd())
                                .unwrap_or(Path::new(&e.os_name))
                                .to_path_buf(),
                        }
                    }
                };
                let Some(w) = cmdline::word(&path, siblings) else {
                    self.say(StatusKind::Error, fl!("cmd-name-ambiguous"));
                    return Some(Task::none());
                };
                Some(w)
            }
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
                                icon: String::new(),
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
                let prefix = self.terminal_prefix();
                let term = terminal.then_some(prefix.as_slice());
                if let Err(e) = spawn_in(&cmdline::argv(&line, &cwd, term), &cwd) {
                    self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                }
                Task::none()
            }
        };
        Task::batch([unfocus(), task])
    }

    /// The terminal chosen in Commands → Terminal, else the system's default.
    pub fn terminal(&self) -> Option<&Terminal> {
        let id = &self.config.terminal_app;
        (self
            .terminals
            .iter()
            .find(|t| !id.is_empty() && t.id == *id))
        .or_else(|| self.term_default.and_then(|i| self.terminals.get(i)))
    }

    /// The dir the terminal follows: the active panel's, on disk (not inside an archive).
    fn term_dir(&self) -> Option<PathBuf> {
        let t = self.panes[self.active].active();
        let cwd = t.panel.cwd();
        (t.pending.is_none() && !archive::within(cwd)).then(|| cwd.to_path_buf())
    }

    /// Start the built-in terminal's shell, if it is on, its channel is up and none runs.
    fn term_start(&mut self) {
        let Some(tx) = self.term_tx.clone().filter(|_| self.config.term_builtin) else {
            return;
        };
        // Not before the panel's dir is read: the shell starts there (a `cd` typed while it
        // still starts would show before its prompt).
        let Some(dir) = self.term_dir().filter(|_| self.term.is_none()) else {
            return;
        };
        let shell = shagoff_core::shell::pick(
            &self.config.term_shell,
            std::env::var("SHELL").ok().as_deref(),
        );
        let (style, dark, bg) = self.term_look();
        match crate::term::start(tx, &shell, Some(&dir), dark, bg, &style) {
            Ok(pane) => self.term = Some(Box::new(pane)),
            // Not again on every message: Ctrl+O or a click on its line tries again.
            Err(e) => {
                self.term_ended = true;
                self.say(
                    StatusKind::Error,
                    fl!("term-failed", err = crate::why::text(&e)),
                );
            }
        }
    }

    /// The terminal's text size (the panels'), and the theme: dark or not, its background.
    fn term_look(&self) -> (crate::term::style::Style, bool, u32) {
        let theme = cosmic::theme::active();
        let c = theme.cosmic();
        let bg = c.bg_color();
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let bg = 0xFF00_0000 | byte(bg.red) << 16 | byte(bg.green) << 8 | byte(bg.blue);
        let style = crate::term::style::Style {
            font_size: f32::from(self.config.panel_size()),
            ..Default::default()
        };
        (style, c.is_dark, bg)
    }

    /// The running terminal takes the panels' text size and the theme now.
    fn term_restyle(&mut self) {
        let (style, dark, bg) = self.term_look();
        if let Some(pane) = &self.term {
            crate::term::restyle(pane, &style, dark, bg);
        }
    }

    /// Rows under the panels: dragged now, else the setting.
    pub fn term_rows(&self) -> u16 {
        match self.term_drag {
            Some(_) => self.term_rows,
            None => self.config.term_rows,
        }
    }

    /// After every `update`: the shell starts once the panel's dir is read, then goes where the
    /// active panel is (`term::follow`).
    fn term_follow(&mut self) {
        if self.term.is_none() {
            if !self.term_ended && self.term_tx.is_some() {
                self.term_start();
            }
            return;
        }
        let dir = self.term_dir();
        if let Some(pane) = &mut self.term {
            match dir {
                Some(dir) => crate::term::follow(pane, &dir),
                None => crate::term::stop_waiting(pane),
            }
        }
    }

    /// Commands → "Shell…": `$SHELL` first, then `/etc/shells`; the chosen one ticked.
    fn shell_items(&self) -> (Vec<ListItem>, usize) {
        let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let list = shagoff_core::shell::shells(&etc);
        let chosen = &self.config.term_shell;
        let at = list.iter().position(|s| s == chosen);
        let env = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let tick = |on: bool, label: String| format!("{}{label}", if on { "✓ " } else { "    " });
        let items = std::iter::once(ListItem {
            label: tick(chosen.is_empty(), fl!("menu-shell-default", name = env)),
            path: PathBuf::new(),
            kind: Item::App,
            icon: String::new(),
        })
        .chain(list.iter().enumerate().map(|(i, s)| ListItem {
            label: tick(at == Some(i), s.clone()),
            path: PathBuf::from(s),
            kind: Item::App,
            icon: String::new(),
        }))
        .collect();
        (items, at.map_or(0, |i| i + 1))
    }

    /// Commands → "Terminal…": the default first, then the installed ones; the chosen one ticked
    /// and under the cursor.
    fn terminal_items(&self) -> (Vec<ListItem>, usize) {
        let id = &self.config.terminal_app;
        let chosen = (self.terminals.iter()).position(|t| !id.is_empty() && t.id == *id);
        let default = match self.term_default.and_then(|i| self.terminals.get(i)) {
            Some(t) => fl!("menu-terminal-default", name = t.name.clone()),
            None => fl!("menu-terminal-none"),
        };
        let tick = |on: bool, label: String| format!("{}{label}", if on { "✓ " } else { "    " });
        let items = std::iter::once(ListItem {
            label: tick(chosen.is_none(), default),
            path: PathBuf::new(),
            kind: Item::App,
            icon: String::new(),
        })
        .chain(self.terminals.iter().enumerate().map(|(i, t)| ListItem {
            label: tick(chosen == Some(i), t.name.clone()),
            path: PathBuf::from(&t.id),
            kind: Item::App,
            icon: String::new(),
        }))
        .collect();
        (items, chosen.map_or(0, |i| i + 1))
    }

    /// Shift+Enter's terminal: `terminal` changed by hand wins (older setups), then the chosen one.
    fn terminal_prefix(&self) -> Vec<String> {
        let by_hand = self.config.terminal != Config::default().terminal;
        match self.terminal().filter(|_| !by_hand) {
            Some(t) => t.run_prefix(),
            None => self.config.terminal.clone(),
        }
    }

    /// Commands → Open terminal: a window in the active panel's dir (inside an archive: the
    /// archive's dir, the one on disk).
    fn open_terminal(&mut self) {
        let cwd = self.panes[self.active].active().panel.cwd().to_path_buf();
        let dir = archive::split_path(&cwd)
            .and_then(|(a, _)| a.parent().map(Path::to_path_buf))
            .unwrap_or(cwd);
        let argv = match self.terminal() {
            Some(t) => t.argv.clone(),
            None => self.config.terminal.iter().take(1).cloned().collect(),
        };
        if let Err(e) = spawn_in(&argv, &dir) {
            let error = format!("{}: {e}", argv.first().map_or("", String::as_str));
            self.say(StatusKind::Error, fl!("cmd-failed", error = error));
        }
    }

    /// Ctrl+↑: the dir under the cursor (an archive as a dir, ".." the parent), else the panel's own.
    fn tab_target(&self, side: usize) -> PathBuf {
        let t = self.panes[side].active();
        let panel = &t.panel;
        // ".." in search results / a branch leads back to the dir they are of (as Backspace).
        if t.results.is_some() && panel.current().is_some_and(|e| e.name == PARENT) {
            return panel.cwd().to_path_buf();
        }
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
        // A hidden Brief / Thumbnails tab missed the resizes: regroup its columns.
        let reveal = if t.brief || t.thumbs {
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

    /// What the viewer covers: none (also in a window of its own), both panes (F3 in place) or
    /// the pane opposite the active one (Ctrl+Q).
    fn viewer_shown(&self) -> Option<Option<usize>> {
        let l = (self.lister.as_deref()).filter(|l| l.quick.is_some() || l.in_place())?;
        Some(l.quick.is_some().then_some(1 - self.active))
    }

    /// A right press while the right button marks: held for `HOLD` it arms the menu.
    fn right_press(&mut self, side: usize, row: Option<usize>) -> Task<Message> {
        const HOLD: std::time::Duration = std::time::Duration::from_millis(500);
        self.right_presses += 1;
        let n = self.right_presses;
        let tab = self.panes[side].active().id;
        self.right_hold = Some(Hold {
            n,
            side,
            tab,
            row,
            armed: false,
            drag: None,
        });
        Task::perform(async { tokio::time::sleep(HOLD).await }, move |()| {
            cosmic::Action::App(Message::RightHeld(n))
        })
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

/// An archive opened from inside another (155) is a copy in the temp dir: a change to it would
/// reach nothing the user keeps — refused, by the one place every archive change goes through.
fn changes_nested(job: &Job) -> bool {
    let tmp = archive::temp_root();
    match job {
        Job::Repack { archive, .. } => archive.starts_with(&tmp),
        Job::Cross {
            from,
            to,
            move_after,
            ..
        } => to.starts_with(&tmp) || (*move_after && from.starts_with(&tmp)),
        _ => false,
    }
}

/// Window events → messages. Panel keys only when no widget took the event (`Ignored`).
fn route_event(
    event: cosmic::iced::Event,
    status: event::Status,
    window: cosmic::iced::window::Id,
) -> Option<Message> {
    match event {
        // The hotkeys dialog waits for a key: the next one, whatever it is (not Escape).
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key,
            modifiers,
            ..
        }) if keymap::CAPTURE.load(std::sync::atomic::Ordering::Relaxed)
            && keymap::key_name(physical_key).is_some() =>
        {
            let b = keymap::bind_of(physical_key, modifiers)?;
            Some(Message::KeyEdit(crate::hotkeys::KeyMsg::Captured(
                b.to_string(),
            )))
        }
        // Any status, any widget under it: the list's own mouse handling takes the release, and a
        // header-edge drag must end wherever the button comes up.
        cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
            cosmic::iced::mouse::Button::Left,
        )) => Some(Message::ColDragEnd),
        cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
            cosmic::iced::mouse::Button::Right,
        )) => Some(Message::RightUp),
        // The divider's share is of the window's width.
        // Ctrl / Shift let go in another window are never seen here: not held any more (a click
        // back in must not mark as Ctrl+click). Not left to the compositor's ModifiersChanged.
        cosmic::iced::Event::Window(cosmic::iced::window::Event::Unfocused) => {
            Some(Message::Modifiers(Modifiers::empty()))
        }
        cosmic::iced::Event::Window(cosmic::iced::window::Event::Closed) => {
            Some(Message::WindowClosed(window))
        }
        cosmic::iced::Event::Window(
            cosmic::iced::window::Event::Resized(size)
            | cosmic::iced::window::Event::Opened { size, .. },
        ) => Some(Message::WindowWidth(window, size.width)),
        // Where files are held, in window coordinates: a zone's own `on_drop` point is shifted by
        // the scrollable it sits in. The right-button drop menu opens at the last one.
        cosmic::iced::Event::Dnd(DndEvent::Offer(
            _,
            OfferEvent::Enter { x, y, .. } | OfferEvent::Motion { x, y },
        )) => Some(Message::DropAt(x, y)),
        // Any status: a focused text_input captures Escape to unfocus itself, and the dialog must still close.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            modifiers,
            ..
        }) if modifiers.is_empty() => Some(Message::Escape(window)),
        // Any status: the search window's fields have focus, and they capture it.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key: physical_key @ keyboard::key::Physical::Code(keyboard::key::Code::KeyL),
            modifiers,
            ..
        }) if modifiers == Modifiers::ALT => Some(Message::FeedKey(
            (status == event::Status::Ignored)
                .then(|| keymap::action(&key, physical_key, modifiers))
                .flatten(),
        )),
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
/// other than text editing and the clipboard (Ctrl+A/C/V/Z, Ctrl+arrows/Backspace/Delete/Home/End);
/// Ctrl+X goes on too: in the command line it is the next command (TC).
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
                Physical::Code(Code::KeyA | Code::KeyC | Code::KeyV | Code::KeyZ)
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
        PlanError::SameTarget(p) => fl!("plan-same-target", path = p.display().to_string()),
        PlanError::Empty => String::new(),
    }
}

/// Why F3 / F4 can't open `file` (shown as `name`), if they can't: a link to nothing says so,
/// anything else (deleted meanwhile, in a dir r-- without x) gives the system's reason.
fn unopenable(file: &Path, name: &str) -> Option<String> {
    let e = std::fs::metadata(file).err()?;
    let link = std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink());
    Some(if link && e.kind() == std::io::ErrorKind::NotFound {
        fl!("broken-link", name = name)
    } else {
        fl!("file-unreadable", name = name, err = crate::why::text(&e))
    })
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
/// `stop`: Esc during Ctrl+B's second step (the rows of millions of found files).
fn read_listing(
    path: &Path,
    show_hidden: bool,
    results: Option<&Vec<PathBuf>>,
    stop: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    match results {
        // A branch read inside an archive: from its index.
        Some(r) if archive::split_path(path).is_some() => Ok(archive::entries(r, path)),
        // Search results fed to the panel: the ones inside archives from their indexes.
        Some(r) if r.iter().any(|p| inside_archive(p)) => {
            let (arc, disk): (Vec<PathBuf>, Vec<PathBuf>) =
                r.iter().cloned().partition(|p| inside_archive(p));
            let mut v = listing::entries(&disk, path, stop);
            v.extend(archive::found_entries(&arc, path));
            Ok(v)
        }
        Some(r) => Ok(listing::entries(r, path, stop)),
        None => listing::scan(path, show_hidden).map_err(|e| crate::why::text(&e)),
    }
}

/// Ctrl+Shift+F5's work, off the UI thread (`ops::link`), its errors as the status line says
/// them. `Ok`: the sources linked and the line for the ones that failed; `Err`: nothing done.
fn link_all(sources: &[PathBuf], target: &Path) -> Result<(Vec<PathBuf>, Option<String>), String> {
    let (done, failed) = ops::link(sources, target).map_err(|e| match e {
        ops::LinkError::Mkdir(path, e) => {
            fl!(
                "link-failed",
                path = path.display().to_string(),
                err = crate::why::text(&e)
            )
        }
        ops::LinkError::Plan(e) => plan_error(&e),
    })?;
    let msg = failed.first().map(|(path, e)| {
        let (path, err) = (path.display().to_string(), crate::why::text(e));
        match failed.len() {
            1 => fl!("link-failed", path = path, err = err),
            n => fl!("links-failed", n = n, path = path, err = err),
        }
    });
    Ok((done, msg))
}

/// Acts on the panel's files (marks, the row under the cursor, a file operation): nothing while a
/// tree (Ctrl+F8) is shown in their place.
fn on_files(a: Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::View
            | A::Edit
            | A::Copy
            | A::CopySame
            | A::Move
            | A::Rename
            | A::Delete
            | A::DeletePermanent
            | A::Mkdir
            | A::NewFile
            | A::Link
            | A::Pack
            | A::Unpack
            | A::TestArchives
            | A::ChecksumCreate
            | A::ChecksumVerify
            | A::Properties
            | A::Occupied
            | A::CountDirs
            | A::Mark
            | A::MarkDown
            | A::MarkUp
            | A::SelectGroup
            | A::UnselectGroup
            | A::ShowAll
            | A::ShowPrograms
            | A::ShowMask
            | A::SelectAll
            | A::UnselectAll
            | A::Invert
            | A::RestoreMarks
            | A::MarkExt
            | A::UnmarkExt
            | A::ClipCut
            | A::ClipCopy
            | A::ClipPaste
            | A::MultiRename
            | A::CopyNames
            | A::CopyPaths
            | A::CmdName
            | A::CmdPath
            | A::ContextMenu
            | A::OpenWith
            | A::OpenLeft
            | A::OpenRight
            | A::Branch
            | A::BranchSel
            | A::CompareLists
            | A::QuickFilter
            | A::QuickView
            | A::Sort(_)
            | A::RunTerminal
    )
}

/// Alt+F10: `dirs`' subdirs read off the UI thread (a dead mount must not hang it), then shown in
/// tree dialog `id`; the cursor on `select` if given (the way there shown even if hidden).
fn tree_read(id: u64, dirs: Vec<PathBuf>, hidden: bool, select: Option<PathBuf>) -> Task<Message> {
    let toward = select.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                shagoff_core::tree::read(dirs, hidden, toward.as_deref())
            })
            .await
            .unwrap_or_default()
        },
        move |read| cosmic::Action::App(Message::TreeFilled(id, read, select.clone())),
    )
}

/// Count dirs `names` in `cwd` in the background; each sends its own `Message::DirSize`.
/// `stop`: the tab left `cwd` (a stopped count sends nothing).
// ponytail: one blocking task per dir; a queue if thousands of dirs ever matter.
fn count_dirs(
    tab: u64,
    cwd: PathBuf,
    names: Vec<OsString>,
    stop: Arc<AtomicBool>,
) -> Task<Message> {
    Task::batch(names.into_iter().map(|name| {
        let path = cwd.join(&name);
        let cwd = cwd.clone();
        let stop = stop.clone();
        cosmic::iced::Task::future(async move {
            tokio::task::spawn_blocking(move || shagoff_core::props::usage(&[path], &stop))
                .await
                .ok()
                .flatten()
        })
        .and_then(move |u| {
            Task::done(cosmic::Action::App(Message::DirSize(
                tab,
                cwd.clone(),
                name.clone(),
                u.bytes,
            )))
        })
    }))
}

/// A menu name shown as is, with no hot letter: `&` markers dropped, plain `&`s kept (doubled).
fn hotkey_free(name: &str) -> String {
    shagoff_core::quicksearch::hotkey(name).0.replace('&', "&&")
}

/// Scroll a list dialog in proportion to its cursor: the row is always in view (separators are
/// shorter, close enough).
fn list_snap(cursor: usize, len: usize) -> Task<Message> {
    let y = cursor as f32 / len.saturating_sub(1).max(1) as f32;
    scrollable::snap_to(
        dialogs::list_scroll_id(),
        scrollable::RelativeOffset {
            x: None,
            y: Some(y),
        },
    )
}

/// "Open with…" rows: the programs, a line, "All programs…" (unless they are all shown already),
/// "Other program…".
fn open_with_items(apps: Vec<openwith::App>, all: bool) -> Vec<ListItem> {
    let own = |label, kind| ListItem {
        label,
        path: PathBuf::new(),
        kind,
        icon: String::new(),
    };
    let mut items: Vec<ListItem> = (apps.into_iter())
        .map(|a| ListItem {
            label: a.name,
            path: a.desktop,
            kind: Item::App,
            icon: a.icon,
        })
        .collect();
    if !items.is_empty() {
        items.push(own(String::new(), Item::Sep));
    }
    if !all {
        items.push(own(fl!("open-with-all"), Item::All));
    }
    items.push(own(fl!("open-with-other"), Item::Other));
    items
}

/// Where a row's file lies: its dir and its own name — not `cwd` for search results fed to the
/// panel, whose rows carry whole paths (157: some inside archives).
fn located(cwd: &Path, os_name: &std::ffi::OsStr) -> (PathBuf, OsString) {
    let full = cwd.join(os_name);
    match (full.parent(), full.file_name()) {
        (Some(d), Some(n)) => (d.to_path_buf(), n.to_os_string()),
        _ => (cwd.to_path_buf(), os_name.to_os_string()),
    }
}

/// A found path through an archive (`/x/a.zip/docs/f`): an ancestor has an archive's name. By
/// name only — a stat per result would stall on a million of them.
fn inside_archive(p: &Path) -> bool {
    p.parent().is_some_and(shagoff_core::archive::within)
}

/// The dialog's search fields as a query (`hidden` is the panel's).
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
    let attrs = shagoff_core::colors::attrs(&f.attrs)
        .ok_or_else(|| fl!("find-bad-attrs", value = f.attrs.trim()))?;
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
        attrs: Some(attrs),
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

/// The installed terminals, read once at start (`.desktop` files and `gio`: not on the UI thread).
fn scan_terminals() -> Task<Message> {
    use i18n_embed::LanguageLoader;
    let lang = (crate::i18n::LANGUAGE_LOADER.current_language().language).to_string();
    Task::perform(
        async move {
            let scan = tokio::task::spawn_blocking(move || terminals::scan(&lang)).await;
            let (list, default) = scan.unwrap_or_default();
            Message::Terminals(list, default)
        },
        cosmic::Action::App,
    )
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

/// Like `spawn_in`, but a failure is reported (`Message::ToolFailed`): a toolbar button's shell
/// always starts, so "command not found" and the like only show in its exit status and stderr.
fn run_watched(argv: &[String], dir: &Path) -> std::io::Result<Task<Message>> {
    use std::io::BufRead;
    use std::process::Stdio;
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut child = std::process::Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let err = child.stderr.take();
    Ok(cosmic::iced::Task::future(async move {
        tokio::task::spawn_blocking(move || {
            // Only the last line is kept: a terminal may log for as long as it is open.
            let last = err.map_or_else(String::new, |e| {
                let lines = std::io::BufReader::new(e).lines().map_while(Result::ok);
                lines
                    .filter(|l| !l.trim().is_empty())
                    .last()
                    .unwrap_or_default()
            });
            let status = child.wait().ok()?;
            (!status.success()).then(|| {
                if last.is_empty() {
                    status.to_string()
                } else {
                    last
                }
            })
        })
        .await
        .ok()
        .flatten()
    })
    .and_then(|e| Task::done(cosmic::Action::App(Message::ToolFailed(e)))))
}

fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || child.wait());
}

/// A font by family name; "" = the interface's.
fn font_named(name: &str) -> cosmic::iced::Font {
    match name.trim() {
        "" => cosmic::font::default(),
        // ponytail: leaks the name once per change of the setting (iced wants `&'static str`).
        n => cosmic::iced::Font {
            family: cosmic::iced::font::Family::Name(Box::leak(n.to_string().into_boxed_str())),
            ..cosmic::font::default()
        },
    }
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
    fn open_with_lists_programs_and_other() {
        let mut app = app_with(Config::default(), State::default());
        // No programs: still "Other program…".
        let _ = app.update(Message::OpenWithApps(0, Vec::new(), Ok(Default::default())));
        let Some(Dialog::List { items, .. }) = &app.dialog else {
            panic!("no list")
        };
        assert_eq!(
            items.iter().map(|i| i.kind).collect::<Vec<_>>(),
            [Item::All, Item::Other]
        );
        app.dialog = None;
        let ed = openwith::App {
            name: "Editor".into(),
            icon: "ed".into(),
            desktop: "/usr/share/applications/ed.desktop".into(),
        };
        let found = openwith::Found {
            mime: "text/plain".into(),
            apps: vec![ed],
        };
        let files = vec![PathBuf::from("/x/a b.txt")];
        let _ = app.update(Message::OpenWithApps(0, files.clone(), Ok(found)));
        let Some(Dialog::List { kind, items, .. }) = &app.dialog else {
            panic!("no list")
        };
        assert_eq!(*kind, ListKind::OpenWith);
        assert_eq!(
            (items[0].label.as_str(), items[0].icon.as_str()),
            ("Editor", "ed")
        );
        let kinds: Vec<Item> = items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [Item::App, Item::Sep, Item::All, Item::Other]);
        // "All programs…" replaces the list, without itself.
        let other = openwith::App {
            name: "Gimp".into(),
            icon: String::new(),
            desktop: "/usr/share/applications/gimp.desktop".into(),
        };
        let _ = app.update(Message::OpenWithAll(vec![other]));
        let Some(Dialog::List { items, .. }) = &app.dialog else {
            panic!("no list")
        };
        let kinds: Vec<Item> = items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [Item::App, Item::Sep, Item::Other]);
        assert_eq!(items[0].label, "Gimp");
        // "Other program…" asks for a command, with the files taken when the list was asked for.
        let _ = app.update(Message::ListPick(2));
        let Some(Dialog::Input { op, sources, .. }) = &app.dialog else {
            panic!("no input")
        };
        assert_eq!((*op, sources.clone()), (InputOp::OpenWith, files));
    }

    #[test]
    fn other_program_remembers_commands() {
        let mut app = app_with(Config::default(), State::default());
        let input = |app: &App| match &app.dialog {
            Some(Dialog::Input { input, .. }) => input.clone(),
            _ => panic!("no input"),
        };
        let found = || Ok(openwith::Found::default());
        // `true` / `false` with no file: nothing to open, the command is still remembered.
        for cmd in ["false", "true"] {
            let _ = app.update(Message::OpenWithApps(0, Vec::new(), found()));
            let _ = app.update(Message::ListPick(1)); // "Other program…"
            let _ = app.update(Message::DialogInput(cmd.into()));
            let _ = app.update(Message::DialogSubmit);
        }
        assert_eq!(app.other_cmds, ["true", "false"]);
        let _ = app.update(Message::OpenWithApps(0, Vec::new(), found()));
        let _ = app.update(Message::ListPick(1));
        assert_eq!(input(&app), "true"); // opens with the last one
        let _ = app.update(Message::FieldKey(Action::Up));
        assert_eq!(input(&app), "false");
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
        // A letter jumps as in a menu, and never reaches the command line.
        let _ = app.update(Message::CmdType('/'));
        let _ = app.update(Message::Key(Action::Up));
        let _ = app.update(Message::CmdType('T'));
        assert!(matches!(app.dialog, Some(Dialog::List { cursor: 1, .. })));
        assert!(app.cmdline.is_empty());
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
    fn a_big_list_is_sorted_by_a_background_read_keeping_cursor_and_marks() {
        use shagoff_core::sort::{Sort, SortKey};
        let mut app = app_with(Config::default(), State::default());
        let dir = PathBuf::from("/big");
        let rows = |n: usize| -> Vec<Entry> {
            (0..n)
                .map(|i| Entry {
                    size: (n - i) as u64, // by size: reversed
                    ..entry(&format!("f{i:05}"))
                })
                .collect()
        };
        let t = app.panes[0].active_mut();
        t.pending = None; // the startup read, never answered here
        let p = &mut t.panel;
        p.set_listing(dir.clone(), rows(SORT_HERE_MAX + 1), None);
        p.set_cursor(2); // "f00001"
        p.toggle_mark();
        let _ = app.update(Message::Key(Action::Sort(SortKey::Size)));
        // Not sorted here: the rows stay, the sort is set, the dir is read again.
        let t = app.panes[0].active();
        assert_eq!(t.panel.sort().key, SortKey::Size);
        assert_eq!(t.panel.entries()[1].name, "f00000");
        assert_eq!(pending_path(&app), Some(dir.clone()));
        let mut sorted = rows(SORT_HERE_MAX + 1);
        let sort = Sort {
            key: SortKey::Size,
            asc: true,
        };
        shagoff_core::sort::sort_entries(&mut sorted, sort, &HashMap::new());
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: dir,
            focus: None,
            result: Ok(sorted),
            space: None,
        });
        let p = &app.panes[0].active().panel;
        assert_eq!(p.entries()[1].name, format!("f{SORT_HERE_MAX:05}"));
        assert_eq!(p.current().unwrap().name, "f00001"); // the cursor stays on its file
        assert_eq!(p.targets(), [PathBuf::from("/big/f00001")]); // still marked
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
    fn regression_a_finished_job_does_not_cancel_navigation() {
        let mut app = app_with(Config::default(), State::default());
        let gone = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let paths = vec![gone.to_path_buf()];
        let job = Job::Delete {
            paths,
            permanent: true,
        };
        let _ = app.start_job(0, OpKind::Delete, job, None);
        let _ = app.update(Message::JobHide); // in the background
        let (tmp, id, _) = navigating(&mut app);
        let _ = app.finish_job(&Report::default()); // it ends while the new dir loads
        assert!(app.job.is_none());
        // Still going there (read again), not back to the dir it left.
        assert_eq!(pending_path(&app).as_deref(), Some(tmp.path()));
        let generation = app.panes[0].active().pending.as_ref().unwrap().0;
        let _ = app.update(listed_ok(id, generation, tmp.path()));
        assert_eq!(app.panes[0].active().panel.cwd(), tmp.path());
    }

    #[test]
    fn regression_watcher_leaves_search_results_alone() {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active_mut();
        t.pending = None;
        t.results = Some((t.panel.cwd().to_path_buf(), Arc::new(Vec::new())));
        let _ = app.update(Message::Changed(0));
        assert_eq!(pending_path(&app), None); // not read again
    }

    #[test]
    fn regression_ctrl_r_before_the_results_arrive_keeps_them() {
        let mut app = app_with(Config::default(), State::default());
        let root = tempfile::tempdir().unwrap();
        let t = app.panes[0].active_mut();
        assert_ne!(t.panel.cwd(), root.path());
        // «В панель»: the results are set, their listing not here yet.
        t.results = Some((root.path().to_path_buf(), Arc::new(Vec::new())));
        t.pending = Some((1, root.path().to_path_buf()));
        let _ = app.update(Message::Key(Action::Reload));
        let t = app.panes[0].active();
        assert!(t.results.is_some());
        assert_eq!(pending_path(&app).as_deref(), Some(root.path()));
    }

    #[test]
    fn regression_ctrl_t_does_not_copy_the_filter() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0]
            .active_mut()
            .panel
            .set_filter(Some("*.rs".into()));
        let _ = app.update(Message::Key(Action::NewTab));
        assert_eq!(app.panes[0].active().panel.filter(), None);
        let _ = app.update(Message::Key(Action::PrevTab));
        assert_eq!(app.panes[0].active().panel.filter(), Some("*.rs")); // the old tab keeps it
    }

    #[test]
    fn regression_click_in_the_other_pane_clears_the_error_like_tab() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().error = Some("no access".into());
        let _ = app.update(Message::Click(1, 0));
        assert_eq!(
            (app.active, app.panes[0].active().error.as_deref()),
            (1, None)
        );
    }

    #[test]
    fn status_card_closes_by_its_cross() {
        let mut app = app_with(Config::default(), State::default());
        app.say(
            StatusKind::Error,
            "a long message that no longer has to fit half a line".into(),
        );
        assert!(app.msg().is_some());
        let _ = app.update(Message::Dismiss(None));
        assert!(app.msg().is_none());
        app.panes[1].active_mut().error = Some("No such file or directory".into());
        let _ = app.update(Message::Dismiss(Some(1)));
        assert!(app.panes[1].active().error.is_none());
    }

    #[test]
    fn regression_alt_left_to_a_deleted_dir_keeps_history_on_the_dir_shown() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(b.join("sub")).unwrap();
        std::fs::create_dir(&a).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &a);
        listed_at(&mut app, 0, &b);
        std::fs::remove_dir(&a).unwrap();
        let _ = app.update(Message::Key(Action::HistoryBack));
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: a.clone(),
            focus: None,
            result: Err("No such file or directory".into()),
            space: None,
        });
        listed_at(&mut app, 0, &b.join("sub")); // Enter
        let recent = app.panes[0].active().history.recent();
        assert_eq!(recent[..2], [b.join("sub"), b.clone()]); // /b is not lost
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
        // Not looked at on the UI thread (a dead mount would hang the start): the tab's first
        // read goes up to an existing dir (`session::existing_dir`, in the background).
        assert_eq!(cwds(&app, 1), [tmp.path().join("gone")]);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn regression_gvfs_dir_is_never_read_on_the_ui_thread() {
        // A hung network mount blocks the gvfs dir: listing it in `update` froze the window.
        let mut app = app_with(Config::default(), State::default());
        let net = Drive {
            label: "net".into(),
            path: "/run/user/1000/gvfs/sftp:host=h".into(),
        };
        app.gvfs_busy = true; // the read is under way
        app.refresh_mounts();
        assert!(app.gvfs_want && !app.drives.contains(&net));
        let _ = app.read_gvfs(); // one at a time: a hung read does not pile up threads
        assert!(app.gvfs_want);
        let _ = app.handle(Message::Gvfs(vec![net.clone()]));
        assert!(!app.gvfs_busy && app.drives.contains(&net));
        app.refresh_mounts(); // kept while the next read runs
        assert!(app.drives.contains(&net));
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
    fn f7_made_in_the_background_rereads_or_says_why() {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let tmp = std::env::temp_dir();
        let _ = app.update(listed_ok(id, generation, &tmp));
        let made = |made| Message::Made {
            side: 0,
            cwd: tmp.clone(),
            name: "new/sub".into(),
            first: Some("new".into()),
            made,
        };
        let _ = app.update(made(Err("Permission denied".into())));
        assert!(app.msg().is_some_and(|m| m.contains("Permission denied")));
        let _ = app.update(made(Ok(())));
        assert_eq!(pending_path(&app), Some(tmp.clone())); // read again, the cursor onto "new"
    }

    #[test]
    fn regression_enter_works_again_while_a_read_hangs() {
        let mut app = app_with(Config::default(), State::default());
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let tmp = std::env::temp_dir();
        let _ = app.update(listed_ok(id, generation, &tmp)); // shows ".."
        let _ = app.load(0, "/dead/mount".into(), None); // never answers
        app.panes[0].active_mut().pending_at -= ENTER_WAIT;
        let _ = app.update(Message::Key(Action::Enter)); // ".." of the dir shown
        assert_eq!(pending_path(&app), tmp.parent().map(Path::to_path_buf));
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

    #[test]
    fn regression_unopenable_says_why_not_always_broken_link() {
        let tmp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("nowhere", tmp.path().join("dangling")).unwrap();
        assert_eq!(
            unopenable(&tmp.path().join("dangling"), "dangling"),
            Some(fl!("broken-link", name = "dangling"))
        );
        // Deleted meanwhile: not a link, the system's reason.
        let gone = unopenable(&tmp.path().join("gone.txt"), "gone.txt").unwrap();
        assert_ne!(gone, fl!("broken-link", name = "gone.txt"));
        assert!(gone.contains("gone.txt"));
        std::fs::write(tmp.path().join("ok"), "x").unwrap();
        assert_eq!(unopenable(&tmp.path().join("ok"), "ok"), None);
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
    fn regression_a_picture_wakes_the_loop_until_the_renderer_has_it() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        let png = tmp.path().join("p.png");
        image::RgbImage::new(4, 4).save(&png).unwrap();
        let l = app.lister.as_mut().unwrap();
        l.name = "p.png".into();
        let id = l.id;
        let ticking = |app: &App| {
            (app.lister.as_ref().and_then(|l| l.wake_until))
                .is_some_and(|t| std::time::Instant::now() < t)
        };
        lister_loaded(&mut app, tmp.path());
        // The renderers' answer may never come while the main window isn't drawn: woken a while.
        assert!(ticking(&app));
        let _ = app.update(Message::ListerAllocated(id, None));
        assert!(
            ticking(&app),
            "failed: the viewer's renderer loads it alone, still woken"
        );
        let until = app.lister.as_ref().unwrap().wake_until.unwrap();
        assert!(until <= std::time::Instant::now() + lister::WAKE_MAX);
        // Text: nothing to wait for.
        let l = app.lister.as_mut().unwrap();
        l.name = "a.txt".into();
        lister_loaded(&mut app, tmp.path());
        assert!(!ticking(&app));
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
    fn f3_opens_a_window_of_its_own_unless_set_otherwise() {
        let (mut app, _tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        let window = app.lister.as_ref().and_then(|l| l.window).unwrap();
        assert!(!app.lister.as_ref().unwrap().in_place());
        // N / P and F3 again: the same window
        let _ = app.update(Message::ListerStep(true));
        assert_eq!(app.lister.as_ref().and_then(|l| l.window), Some(window));
        // the window closed by the compositor / its ×: the viewer goes too
        assert!(matches!(
            app.on_close_requested(window),
            Some(Message::ListerClose)
        ));
        let _ = app.update(Message::WindowClosed(window));
        assert!(app.lister.is_none());
        // F3 over Ctrl+Q's viewer: its own window all the same
        let _ = app.update(Message::Key(Action::QuickView));
        let _ = app.update(Message::Key(Action::View));
        assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
        // F3 in the other pane with the window open (from the main window, as F3 there does):
        // N / P walk that pane now
        let file = app.panes[1].active().panel.cwd().join("x");
        let _ = app.open_lister(1, file, "x".into());
        assert_eq!(
            app.lister.as_ref().map(|l| (l.side, l.window.is_some())),
            Some((1, true))
        );
        let (mut app, _tmp) = lister_app(Config {
            lister_window: false,
            ..Config::default()
        });
        let _ = app.update(Message::Key(Action::View));
        assert!(app.lister.as_ref().unwrap().in_place());
    }

    #[test]
    fn regression_a_viewer_window_covers_no_pane() {
        // F3 in a window of its own: the panels stay on screen (their thumbnails still load).
        let (mut app, _tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::View));
        assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
        assert_eq!(app.viewer_shown(), None);
        let (mut app, _tmp) = lister_app(Config {
            lister_window: false,
            ..Config::default()
        });
        let _ = app.update(Message::Key(Action::View));
        assert_eq!(app.viewer_shown(), Some(None)); // in place: both covered
    }

    #[test]
    fn ctrl_q_follows_the_cursor_and_leaves_keys_to_the_panels() {
        let (mut app, tmp) = lister_app(Config::default());
        let _ = app.update(Message::Key(Action::QuickView));
        let shown = |app: &App| app.lister.as_ref().map(|l| l.name.clone());
        assert_eq!(shown(&app).as_deref(), Some("a.txt"));
        lister_loaded(&mut app, tmp.path());
        assert_eq!(app.lister.as_ref().unwrap().rows(), 2);
        // ↓ moves the panel's cursor, and the view follows it.
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(shown(&app).as_deref(), Some("b.bin"));
        // A dir: a note, no contents.
        let _ = app.update(Message::Key(Action::Up));
        let _ = app.update(Message::Key(Action::Up));
        let l = app.lister.as_ref().unwrap();
        assert_eq!(l.name, "sub");
        // A dir is counted (TC: its size), and the count stops once the cursor leaves it.
        assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
        let counting = app.quick_stop.clone();
        let _ = app.update(Message::Key(Action::Up)); // `..`
        assert!(counting.load(Ordering::Relaxed));
        let l = app.lister.as_ref().unwrap();
        assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-none")));
        let _ = app.update(Message::Key(Action::Down));
        // F8 is the panel's: it asks to delete.
        let _ = app.update(Message::Key(Action::Delete));
        assert!(app.dialog.is_some());
        let _ = app.update(Message::DialogCancel);
        assert!(app.lister.is_some()); // Esc closed the dialog, not the view
        let counting = app.quick_stop.clone();
        let _ = app.update(Message::Key(Action::QuickView));
        assert!(app.lister.is_none());
        assert!(counting.load(Ordering::Relaxed), "closed: its count stops");
    }

    #[test]
    fn ctrl_q_inside_an_archive_reads_files_and_notes_dirs() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let z = tmp.path().join("a.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        w.add_directory("d/", opts).unwrap();
        w.start_file("f.txt", opts).unwrap();
        w.write_all(b"inside").unwrap();
        w.start_file("g.bin", opts).unwrap(); // past 32 MB (zeros pack small)
        w.write_all(&vec![0; (shagoff_core::lister::LIMIT + 1) as usize])
            .unwrap();
        w.finish().unwrap();
        let mut app = app_with(Config::default(), State::default());
        let _ = app.load(0, z.clone(), None);
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: z.clone(),
            focus: Some("d".into()),
            result: Ok(listing::scan(&z, false).unwrap()),
            space: None,
        });
        let _ = app.update(Message::Key(Action::QuickView));
        let l = app.lister.as_ref().unwrap();
        assert_eq!(l.name, "d");
        // Counted from the archive's index, nothing unpacked.
        assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
        let u = archive::usage(&z, Path::new("d")).unwrap();
        assert_eq!((u.files, u.dirs), (0, 1));
        let _ = app.update(Message::Key(Action::Down));
        let l = app.lister.as_ref().unwrap();
        assert_eq!(l.name, "f.txt");
        assert!(l.loaded.is_none(), "being taken out and read, not refused");
        // Too big to unpack whole for a 32 MB look.
        let _ = app.update(Message::Key(Action::Down));
        let l = app.lister.as_ref().unwrap();
        assert_eq!(l.name, "g.bin");
        assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if t.contains("F3")));
        // What the background read does for it.
        let stop = std::sync::atomic::AtomicBool::new(false);
        let out = tmp.path().join("out");
        let f = archive::extract_one(&z, Path::new(""), Path::new("f.txt"), &out, &stop).unwrap();
        assert_eq!(std::fs::read_to_string(f).unwrap(), "inside");
    }

    #[test]
    fn regression_ctrl_q_reloads_a_changed_file_and_shows_a_zip() {
        let (mut app, tmp) = lister_app(Config::default());
        let rescan = |app: &mut App, focus: &str| {
            let _ = app.load(0, tmp.path().into(), Some(focus.into()));
            let t = app.panes[0].active();
            let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
            let _ = app.update(Message::Listed {
                tab: id,
                generation,
                path: tmp.path().into(),
                focus: Some(focus.into()),
                result: Ok(listing::scan(tmp.path(), false).unwrap()),
                space: None,
            });
        };
        let _ = app.update(Message::Key(Action::QuickView));
        let before = app.lister.as_ref().unwrap().id;
        std::fs::write(tmp.path().join("a.txt"), "changed, and longer\n").unwrap();
        rescan(&mut app, "a.txt");
        assert_ne!(
            app.lister.as_ref().unwrap().id,
            before,
            "same name, new size: read again"
        );
        // A zip on disk is no path inside an archive: shown like a folder, what it holds.
        std::fs::write(tmp.path().join("c.zip"), b"PK").unwrap();
        rescan(&mut app, "c.zip");
        let l = app.lister.as_ref().unwrap();
        assert_eq!(l.name, "c.zip");
        assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
    }

    #[test]
    fn ctrl_b_lists_every_file_below_and_again_leaves() {
        let (mut app, tmp) = lister_app(Config::default());
        std::fs::write(tmp.path().join("sub/deep.txt"), "d").unwrap();
        let tab = app.panes[0].active().id;
        let root = tmp.path().to_path_buf();
        let listed = |app: &mut App, root: PathBuf| {
            let paths =
                listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
            let id = app.panes[0].active().branching.unwrap_or(0);
            let _ = app.update(Message::BranchListed {
                tab,
                id,
                root,
                paths,
            });
        };
        let _ = app.update(Message::Key(Action::Branch)); // reads in the background
        listed(&mut app, root.clone());
        let t = app.panes[0].active();
        assert!(t.branch);
        let mut found: Vec<String> = (t.results.as_ref().unwrap().1.iter())
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        found.sort();
        assert_eq!(found, ["a.txt", "b.bin", "sub/deep.txt"]);
        // Ctrl+B again: the dir as usual.
        let _ = app.update(Message::Key(Action::Branch));
        assert!(app.panes[0].active().results.is_none());
        // A read that comes back after the tab went elsewhere — even back here — is dropped.
        let _ = app.update(Message::Key(Action::Branch));
        let id = app.panes[0].active().branching.unwrap();
        let _ = app.load(0, root.join("sub"), None);
        let _ = app.load(0, root.clone(), None);
        let paths = listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
        let _ = app.update(Message::BranchListed {
            tab,
            id,
            root: root.clone(),
            paths,
        });
        assert!(app.panes[0].active().results.is_none());
        // Leaving the dir stops the read too.
        let _ = app.update(Message::Key(Action::Branch));
        let stop = app.panes[0].active().branch_stop.clone();
        let _ = app.load(0, root.join("sub"), None);
        assert!(stop.load(Ordering::Relaxed));
    }

    #[test]
    fn esc_stops_the_second_step_of_a_branch_too() {
        let (mut app, tmp) = lister_app(Config::default());
        let root = tmp.path().to_path_buf();
        let tab = app.panes[0].active().id;
        let branch = |app: &mut App| {
            let _ = app.update(Message::Key(Action::Branch));
            let t = app.panes[0].active();
            let id = t.branching.unwrap();
            let paths =
                listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
            let _ = app.update(Message::BranchListed {
                tab,
                id,
                root: root.clone(),
                paths,
            });
        };
        branch(&mut app);
        // Found; the rows (one lstat each) are being read: still under way, Esc can stop it.
        let t = app.panes[0].active();
        assert!(t.branching.is_some() && t.results.is_some());
        let stop = t.branch_stop.clone();
        let _ = app.update(Message::DialogCancel); // Esc
        assert!(stop.load(Ordering::Relaxed));
        let t = app.panes[0].active();
        assert_eq!(t.branching, None);
        assert!(t.results.is_none()); // back to the dir as usual
        assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root.clone()));
        // Read to the end: no longer under way, the status cleared.
        branch(&mut app);
        let t = app.panes[0].active();
        let generation = t.pending.as_ref().unwrap().0;
        let (_, paths) = t.results.clone().unwrap();
        let rows = read_listing(&root, false, Some(&paths), &AtomicBool::new(false));
        let _ = app.update(Message::Listed {
            tab,
            generation,
            path: root.clone(),
            focus: None,
            result: rows,
            space: None,
        });
        let t = app.panes[0].active();
        assert_eq!(t.branching, None);
        assert!(t.results.is_some());
        assert!(app.status.is_none());
    }

    #[test]
    fn esc_stops_a_branch_read() {
        let (mut app, tmp) = lister_app(Config::default());
        let tab = app.panes[0].active().id;
        let _ = app.update(Message::Key(Action::Branch));
        let (id, stop) = {
            let t = app.panes[0].active();
            (t.branching.unwrap(), t.branch_stop.clone())
        };
        let _ = app.update(Message::DialogCancel); // Esc
        assert!(stop.load(Ordering::Relaxed));
        assert_eq!(app.panes[0].active().branching, None);
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Info));
        // What the worker sends back afterwards (stopped, or done just before) is dropped.
        let _ = app.update(Message::BranchListed {
            tab,
            id,
            root: tmp.path().to_path_buf(),
            paths: Some(Vec::new()),
        });
        assert!(app.panes[0].active().results.is_none());
        // Another tab's read: Esc stops it too; closing a tab stops its own.
        let _ = app.update(Message::Key(Action::Branch));
        let first = app.panes[0].active().branch_stop.clone();
        let _ = app.update(Message::Key(Action::NewTab));
        let _ = app.update(Message::DialogCancel);
        assert!(first.load(Ordering::Relaxed));
        let _ = app.update(Message::Key(Action::Branch));
        let second = app.panes[0].active().branch_stop.clone();
        let _ = app.update(Message::Key(Action::CloseTab));
        assert!(second.load(Ordering::Relaxed));
    }

    #[test]
    fn esc_during_a_drag_ends_it_and_nothing_else() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().panel.set_filter(Some("x".into()));
        let _ = app.update(Message::DragStart(Some(Arc::new(vec![PathBuf::from(
            "/a",
        )]))));
        let _ = app.update(Message::Escape(app.window_id()));
        assert!(!app.dragging && app.right_drag.is_none());
        assert!(app.panes[0].active().panel.filter().is_some()); // the drag's Esc, not the filter's
        let _ = app.update(Message::Escape(app.window_id()));
        assert!(app.panes[0].active().panel.filter().is_none());
        // A drag that ended (dropped) leaves Esc to the panels again.
        let _ = app.update(Message::DragStart(None));
        let _ = app.update(Message::DragEnd);
        assert!(!app.dragging);
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
    fn regression_viewer_n_in_an_archive_cancelled_keeps_the_cursor_on_the_shown_file() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
        for n in ["a.txt", "b.txt"] {
            z.start_file(n, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
        }
        z.finish().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down)); // a.txt
        let cur = |app: &App| app.panes[0].active().panel.current().unwrap().name.clone();
        // F3: extracted, then shown.
        let _ = app.update(Message::Key(Action::View));
        let file = app
            .job
            .as_ref()
            .and_then(|j| j.open.as_ref())
            .unwrap()
            .1
            .clone();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x").unwrap();
        let done = Report {
            cancelled: false,
            completed: vec![a.join("a.txt")],
        };
        let _ = app.update(Message::Op(jobs::Event::Finished(Arc::new(done))));
        assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
        // N: b.txt is being extracted, the cursor already on it; Esc stops that.
        let _ = app.update(Message::ListerKey(ListerKey::Next));
        assert_eq!(cur(&app), "b.txt");
        let stopped = Report {
            cancelled: true,
            completed: Vec::new(),
        };
        let _ = app.update(Message::Op(jobs::Event::Finished(Arc::new(stopped))));
        assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
        assert_eq!(cur(&app), "a.txt"); // with the file on screen
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
        // 21-char names: Brief columns of ~195 px, two on the 500 px list.
        let entries = (0..20)
            .map(|i| entry(&format!("file-with-long-name{i:02}")))
            .collect();
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
    fn thumbs_view_moves_by_cells_and_rows() {
        // ".." + 20 files; 500 px = 4 cells a row
        let mut app = tall_list(4.5 * ROW);
        let cur = |app: &App| app.panes[0].active().panel.cursor();
        let _ = app.update(Message::Key(Action::ViewBrief));
        let _ = app.update(Message::Key(Action::ViewThumbs));
        let t = app.panes[0].active();
        assert!(t.thumbs && !t.brief);
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), 1);
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(cur(&app), 5);
        let _ = app.update(Message::Key(Action::PageDown)); // one row on screen
        assert_eq!(cur(&app), 9);
        let _ = app.update(Message::Key(Action::Up));
        let _ = app.update(Message::Key(Action::Left));
        assert_eq!(cur(&app), 4);
        let _ = app.update(Message::Key(Action::End));
        let t = app.panes[0].active();
        let cell_h = crate::thumbs::cell_h(app.row_h());
        assert!(t.offset + t.height >= 6.0 * cell_h); // the last row (6th) on screen
        let _ = app.update(Message::Key(Action::ViewFull));
        assert!(!app.panes[0].active().thumbs);
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
    fn toolbar_shift_drag_moves_a_button_without_running_it() {
        let mut app = app_with(Config::default(), State::default());
        let bar = app.config.toolbar.clone();
        let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
        let _ = app.update(Message::ToolHover(Some(1)));
        let _ = app.update(Message::ToolGrab);
        let _ = app.update(Message::ToolHover(Some(2)));
        let _ = app.update(Message::Tool(2)); // the release over it: not run
        let _ = app.update(Message::ToolRelease);
        assert!(!app.panes[0].active().brief, "cm_SrcShort not run");
        assert_eq!(app.config.toolbar[2], bar[1]);
        assert_eq!(app.config.toolbar[1], bar[2]);
        // Released: hovering moves nothing.
        let _ = app.update(Message::ToolHover(Some(0)));
        assert_eq!(app.config.toolbar[2], bar[1]);
        // Shift let go mid-drag (the release may never come, e.g. focus left): dropped there.
        let _ = app.update(Message::ToolGrab);
        let _ = app.update(Message::Modifiers(Modifiers::empty()));
        let _ = app.update(Message::ToolHover(Some(1)));
        assert_eq!(app.config.toolbar[0], bar[0]);
        // Under a dialog nothing is grabbed.
        let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
        let _ = app.update(Message::Key(Action::ConfigureToolbar));
        let _ = app.update(Message::ToolGrab);
        assert_eq!(app.tool_drag.grab, None);
    }

    #[test]
    fn toolbar_buttons_run_and_the_bar_is_edited() {
        use crate::toolbar::ToolMsg;
        let mut app = app_with(Config::default(), State::default());
        let brief = (app.config.toolbar.iter())
            .position(|b| b.cmd == "cm_SrcShort")
            .unwrap();
        let _ = app.update(Message::Tool(brief));
        assert!(app.panes[0].active().brief);
        let _ = app.update(Message::Key(Action::ConfigureToolbar));
        let _ = app.update(Message::ToolEdit(ToolMsg::Select(0)));
        let _ = app.update(Message::ToolEdit(ToolMsg::Delete));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        assert_eq!(app.config.toolbar, crate::toolbar::default_bar()[1..]);
    }

    #[test]
    fn a_tab_dropped_on_the_other_pane_moves_there_or_is_copied_with_ctrl() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::NewTab)); // pane 0: two tabs
        for t in app.panes[0].items_mut() {
            t.width = 500.0; // the right pane starts past 508
        }
        let count = |app: &App| (app.panes[0].items().len(), app.panes[1].items().len());
        // Released over its own pane: nothing.
        let _ = app.update(Message::TabPress(0, 1));
        let _ = app.update(Message::TabDragAt(app.window_id(), 10.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(count(&app), (2, 1));
        // Over the other pane: moved there, that pane active.
        let _ = app.update(Message::TabPress(0, 1));
        let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!((count(&app), app.active), ((1, 2), 1));
        // A pointer that only passes over the other pane without a tab held: nothing.
        let _ = app.update(Message::TabDragAt(app.window_id(), 10.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(count(&app), (1, 2));
        // With Ctrl: copied; the last tab of a pane can be copied too.
        app.mods = Modifiers::CTRL;
        let _ = app.update(Message::TabPress(0, 0));
        let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(count(&app), (1, 3));
        // The split between the panes, for both skins: the middle of the gap.
        assert_eq!(
            crate::view::pane_split(crate::config::Skin::Classic, 500.0, false),
            502.0
        );
        // the classic frame: its 1 px inset on each side of the left pane
        assert_eq!(
            crate::view::pane_split(crate::config::Skin::Classic, 500.0, true),
            504.0
        );
        assert_eq!(
            crate::view::pane_split(crate::config::Skin::Modern, 500.0, false),
            524.0
        );
        // A popup's coordinates are not the main window's.
        let _ = app.update(Message::TabPress(0, 0));
        let popup = cosmic::iced::window::Id::unique();
        let _ = app.update(Message::TabDragAt(popup, 700.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(count(&app), (1, 3));
    }

    #[test]
    fn regression_a_viewer_window_does_not_stop_a_tab_drop() {
        let (mut app, _tmp) = lister_app(Config::default());
        app.active = 0;
        let _ = app.update(Message::Key(Action::NewTab)); // pane 0: two tabs
        let _ = app.update(Message::Key(Action::View)); // F3 in its own window
        assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
        for t in app.panes[0].items_mut() {
            t.width = 500.0;
        }
        let before = app.panes[1].items().len();
        let _ = app.update(Message::TabPress(0, 1));
        let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(app.panes[1].items().len(), before + 1);
    }

    #[test]
    fn tabs_move_by_drag_and_keys() {
        let mut app = app_with(Config::default(), State::default());
        for _ in 0..2 {
            let _ = app.update(Message::Key(Action::NewTab));
        }
        for (i, t) in app.panes[0].items_mut().iter_mut().enumerate() {
            t.name = Some(i.to_string());
        }
        let names = |app: &App| -> String {
            (app.panes[0].items().iter())
                .map(|t| t.name.clone().unwrap())
                .collect()
        };
        let _ = app.update(Message::TabEnter(0, 1)); // no drag: nothing moves
        assert_eq!(names(&app), "012");
        let _ = app.update(Message::TabPress(0, 0));
        let _ = app.update(Message::TabEnter(0, 1));
        let _ = app.update(Message::TabEnter(0, 2));
        let _ = app.update(Message::ColDragEnd);
        let _ = app.update(Message::TabEnter(0, 0)); // released
        assert_eq!(
            (names(&app), app.panes[0].active_index()),
            ("120".into(), 2)
        );
        let _ = app.update(Message::Key(Action::MoveTabRight)); // last: stays
        let _ = app.update(Message::Key(Action::MoveTabLeft));
        assert_eq!(
            (names(&app), app.panes[0].active_index()),
            ("102".into(), 1)
        );
    }

    #[test]
    fn wheel_scroll_does_not_drag_the_view_back_to_the_cursor() {
        let mut app = tall_list(100.0);
        let _ = app.update(Message::Scrolled(0, 200.0, 100.0)); // user scrolled away
        assert_eq!(app.panes[0].active().offset, 200.0);
    }

    /// Pane 0 shows `names` (files) in temp_dir.
    #[test]
    fn ctrl_arrows_open_in_that_panel_and_shift_f10_is_the_row_menu() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("f.txt"), "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // "sub"
        let _ = app.update(Message::Key(Action::OpenRight));
        let pending = |app: &App, s: usize| app.panes[s].active().pending.clone().map(|(_, p)| p);
        assert_eq!(pending(&app, 1), Some(tmp.path().join("sub")));
        let _ = app.update(Message::Key(Action::Down)); // "f.txt": this panel's dir
        let _ = app.update(Message::Key(Action::OpenRight));
        assert_eq!(pending(&app, 1).as_deref(), Some(tmp.path()));
        // On a file, Ctrl+← in its own (left) panel: nothing read again.
        let before = pending(&app, 0);
        let _ = app.update(Message::Key(Action::OpenLeft));
        assert_eq!(pending(&app, 0), before);
        // Shift+F10 on the file: its menu as a list; a pick does what the item does.
        let _ = app.update(Message::Key(Action::ContextMenu));
        let Some(Dialog::List {
            kind: ListKind::Context,
            items,
            ..
        }) = &app.dialog
        else {
            panic!("no menu");
        };
        let mkdir_free = !items
            .iter()
            .any(|it| it.kind == Item::Act(crate::menu::MenuAct::Key(Action::Mkdir)));
        let props = (items.iter())
            .position(|it| it.kind == Item::Act(crate::menu::MenuAct::Key(Action::Properties)))
            .unwrap();
        assert!(mkdir_free && items.iter().any(|it| it.kind == Item::Sep));
        let _ = app.update(Message::ListPick(props));
        assert!(matches!(app.dialog, Some(Dialog::Props(_))));
    }

    #[test]
    fn regression_dialog_arrows_walk_its_buttons_not_the_panel() {
        let mut app = files_app(&["a.txt", "b.txt"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Delete));
        assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
        let cursor = app.panes[0].active().panel.cursor();
        for a in [
            Action::Down,
            Action::Right,
            Action::Up,
            Action::Left,
            Action::SwitchPane,
        ] {
            let _ = app.update(Message::Key(a)); // focus moves (a widget operation)
        }
        assert_eq!(app.panes[0].active().panel.cursor(), cursor);
        assert_eq!(app.active, 0);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
    }

    #[test]
    fn alt_shift_f9_tests_the_archives_and_says_what_is_wrong() {
        let mut app = files_app(&["notes.txt"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::TestArchives)); // no archive here
        assert_eq!(app.msg(), Some(fl!("test-no-archives").as_str()));
        let ok = (PathBuf::from("/x/a.zip"), Ok(3));
        let _ = app.update(Message::Tested(vec![ok.clone()]));
        assert!(app.msg().is_some_and(|m| m.contains('3')));
        let bad = (PathBuf::from("/x/b.7z"), Err("checksum".to_string()));
        let _ = app.update(Message::Tested(vec![ok, bad]));
        assert!(app.msg().is_some_and(|m| m.contains("b.7z: checksum")));
    }

    #[test]
    fn ctrl_f10_f11_f12_show_all_programs_or_by_mask() {
        let mut app = files_app(&["a.rs", "b.txt"]);
        let shown = |app: &App| app.panes[0].active().panel.entries().len();
        let _ = app.update(Message::Key(Action::ShowPrograms)); // none has an x bit
        assert_eq!(shown(&app), 1, "only ..");
        let _ = app.update(Message::Key(Action::ShowMask));
        let Some(Dialog::Mask {
            op: MaskOp::Show, ..
        }) = &app.dialog
        else {
            panic!("no mask dialog");
        };
        let _ = app.update(Message::DialogInput("*.rs".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(
            app.panes[0].active().panel.show(),
            &Show::Mask("*.rs".into())
        );
        assert_eq!(shown(&app), 2);
        let _ = app.update(Message::Key(Action::ShowMask)); // offers the mask in force
        assert!(matches!(&app.dialog, Some(Dialog::Mask { input, .. }) if input == "*.rs"));
        let _ = app.update(Message::DialogInput("*.*".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.panes[0].active().panel.show(), &Show::All);
        let _ = app.update(Message::Key(Action::ShowPrograms));
        let _ = app.update(Message::Key(Action::ShowAll));
        assert_eq!(shown(&app), 3);
    }

    #[test]
    fn checksums_dialog_keeps_the_algo_and_verify_says_what_is_wrong() {
        let mut app = files_app(&["notes.txt"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::ChecksumVerify)); // no sum file
        assert_eq!(app.msg(), Some(fl!("checksum-no-files").as_str()));
        let _ = app.update(Message::Key(Action::ChecksumCreate));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Checksum {
                algo: Algo::Sha256,
                ..
            })
        ));
        let _ = app.update(Message::SumAlgo(Algo::Md5));
        assert_eq!(app.config.checksum_algo, "md5");
        assert!(matches!(
            app.dialog,
            Some(Dialog::Checksum {
                algo: Algo::Md5,
                ..
            })
        ));
        let _ = app.update(Message::Key(Action::Enter)); // no text field: Enter creates
        assert!(app.dialog.is_none());
        let sum = PathBuf::from("/x/SHA256SUMS");
        let good = vec![(PathBuf::from("a"), Check::Ok)];
        let _ = app.update(Message::Verified(vec![(sum.clone(), Ok(good.clone()))]));
        assert_eq!(app.msg(), Some(fl!("checksum-ok", n = 1).as_str()));
        let bad = vec![(PathBuf::from("b"), Check::Bad)];
        let _ = app.update(Message::Verified(vec![(sum, Ok([good, bad].concat()))]));
        assert!(
            app.msg()
                .is_some_and(|m| m.contains(&format!("b: {}", fl!("checksum-bad"))))
        );
    }

    #[test]
    fn alt_f10_tree_opens_the_way_to_here_and_goes_where_chosen() {
        use shagoff_core::tree::{chain, read};
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("inner")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::TreeDialog));
        let id = match &app.dialog {
            Some(Dialog::Tree { id, .. }) => *id,
            _ => panic!("no tree"),
        };
        // What its background read brings: the way down from `/`, the cursor on the dir here.
        let got = read(chain(Path::new("/"), tmp.path()), false, Some(tmp.path()));
        let _ = app.update(Message::TreeFilled(id, got, Some(tmp.path().into())));
        let cur = |app: &App| match &app.dialog {
            Some(Dialog::Tree { tree, .. }) => tree.current().unwrap().path.clone(),
            _ => panic!("no tree"),
        };
        assert_eq!(cur(&app), tmp.path());
        // → reads it (here: what that read brings), → again steps in.
        let _ = app.update(Message::Key(Action::Right));
        let inner = vec![(tmp.path().to_path_buf(), vec![OsString::from("inner")])];
        let _ = app.update(Message::TreeFilled(id + 1, inner.clone(), None)); // another dialog's
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), tmp.path()); // not read yet: nothing to step into
        let _ = app.update(Message::TreeFilled(id, inner, None));
        let _ = app.update(Message::Key(Action::Right));
        assert_eq!(cur(&app), tmp.path().join("inner"));
        let _ = app.update(Message::Key(Action::Left)); // closed: to its parent
        assert_eq!(cur(&app), tmp.path());
        let _ = app.update(Message::Key(Action::Left)); // open: closed
        let _ = app.update(Message::Key(Action::Left)); // to its parent
        assert_eq!(cur(&app), tmp.path().parent().unwrap());
        let _ = app.update(Message::Key(Action::Enter));
        assert!(app.dialog.is_none());
        let pending = app.panes[0].active().pending.clone().map(|(_, p)| p);
        assert_eq!(pending.as_deref(), tmp.path().parent());
    }

    #[test]
    fn ctrl_f8_tree_in_the_panel_leads_the_other_one() {
        use shagoff_core::tree::{chain, read};
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("a")).unwrap();
        std::fs::create_dir(tmp.path().join("b")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::TreePanel));
        let id = app.panes[0].active().tree.as_ref().map(|pt| pt.id).unwrap();
        let mut got = read(chain(Path::new("/"), tmp.path()), false, Some(tmp.path()));
        got.extend(read(vec![tmp.path().into()], false, None)); // and its own subdirs
        let _ = app.update(Message::TreeFilled(id, got, Some(tmp.path().into())));
        let cur = |app: &App| {
            let pt = app.panes[0].active().tree.as_ref().unwrap();
            pt.tree.current().unwrap().path.clone()
        };
        assert_eq!(cur(&app), tmp.path());
        // ↓: onto "a"; the other panel goes there.
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(cur(&app), tmp.path().join("a"));
        let pending = |app: &App| app.panes[1].active().pending.clone().map(|(_, p)| p);
        assert_eq!(pending(&app), Some(tmp.path().join("a")));
        // F8 under the tree: no files there to delete.
        let _ = app.update(Message::Key(Action::Delete));
        assert!(app.dialog.is_none());
        // Not about the files: as usual (Ctrl+T, then back).
        let _ = app.update(Message::Key(Action::NewTab));
        assert_eq!(app.panes[0].items().len(), 2);
        assert!(app.panes[0].active().tree.is_none()); // the new tab: files
        let _ = app.update(Message::Key(Action::CloseTab));
        assert!(app.panes[0].active().tree.is_some());
        // The tree's tab sent elsewhere (Alt+F10…): its tree opened again there.
        let before = app.panes[0].active().tree.as_ref().unwrap().id;
        listed_at(&mut app, 0, &tmp.path().join("b"));
        assert_ne!(app.panes[0].active().tree.as_ref().unwrap().id, before);
        // Enter: on to the files.
        let _ = app.update(Message::Key(Action::Enter));
        assert_eq!(app.active, 1);
        app.active = 0;
        let _ = app.update(Message::Key(Action::TreePanel)); // off
        assert!(app.panes[0].active().tree.is_none());
    }

    #[test]
    fn ctrl_l_says_the_occupied_space() {
        let mut app = app_with(Config::default(), State::default());
        let u = shagoff_core::props::Usage {
            bytes: 2048,
            files: 3,
            dirs: 1,
        };
        let _ = app.update(Message::Occupied(Some(u)));
        assert!(
            app.msg()
                .is_some_and(|m| m.contains('3') && m.contains('1'))
        );
    }

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
    fn ctrl_shift_f5_links_the_file_into_the_other_pane() {
        let (tmp, mut app, a) = paste_setup();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        listed_at(&mut app, 0, &src);
        listed_at(&mut app, 1, &dst);
        app.panes[0].active_mut().panel.set_cursor(1); // "a", after ".."
        let _ = app.update(Message::Key(Action::Link));
        let Some(Dialog::Input {
            op: InputOp::Link,
            input,
            ..
        }) = &app.dialog
        else {
            panic!("no link dialog");
        };
        assert_eq!(input, &dir_input(&dst));
        let target = PathBuf::from(input);
        let _ = app.update(Message::DialogSubmit);
        assert!(app.dialog.is_none());
        // What its background task does, then its message.
        let _ = app.update(Message::Linked(
            0,
            link_all(std::slice::from_ref(&a), &target),
        ));
        assert_eq!(std::fs::read_link(dst.join("a")).unwrap(), a);
        // Again: the name is taken.
        let _ = app.update(Message::Linked(
            0,
            link_all(std::slice::from_ref(&a), &target),
        ));
        assert!(app.msg().is_some_and(|m| m.contains("dst/a")));
    }

    #[test]
    fn shift_f5_with_marks_offers_a_mask_for_all_of_them() {
        let mut app = files_app(&["a.txt", "b.txt"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::MarkDown));
        let _ = app.update(Message::Key(Action::MarkDown));
        let _ = app.update(Message::Key(Action::CopySame));
        let Some(Dialog::Input {
            op, input, sources, ..
        }) = &app.dialog
        else {
            panic!("no dialog");
        };
        assert_eq!((*op, input.as_str()), (InputOp::Copy, "*.*"));
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn archive_password_error_asks_and_retries_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("e.zip");
        let mut app = app_with(Config::default(), State::default());
        let (reply, rx) = std::sync::mpsc::channel();
        let _ = app.on_job_event(jobs::Event::Error {
            path: a.clone(),
            error: archive::PASSWORD_ERROR.into(),
            reply,
        });
        assert!(matches!(
            app.dialog,
            Some(Dialog::Password { wrong: false, .. })
        ));
        let _ = app.update(Message::ArchivePassword("pw".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(rx.try_recv(), Ok(ErrorChoice::Retry));
        assert_eq!(archive::password(&a).as_deref(), Some("pw"));
        // Asked again: the one given was wrong; Esc cancels the job.
        let (reply, rx) = std::sync::mpsc::channel();
        let _ = app.on_job_event(jobs::Event::Error {
            path: a.clone(),
            error: archive::PASSWORD_ERROR.into(),
            reply,
        });
        assert!(matches!(
            app.dialog,
            Some(Dialog::Password { wrong: true, .. })
        ));
        let _ = app.update(Message::DialogCancel);
        assert_eq!(rx.try_recv(), Ok(ErrorChoice::Cancel));
        archive::set_password(&a, None);
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
    fn regression_alt_letter_adds_to_the_open_search() {
        let mut app = files_app(&["alpha", "beta", "bravo"]);
        let _ = app.update(Message::Key(Action::QuickSearch('b')));
        // Unfocused, the field's typing comes as keys: Alt+r adds to "b", not starts over.
        let _ = app.update(Message::Key(Action::QuickSearch('r')));
        assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("br"));
        assert_eq!(cursor_name(&app), "bravo");
        let _ = app.update(Message::Key(Action::QuickSearch('z'))); // no match: not taken
        assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("br"));
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
    fn regression_hotlist_delete_after_an_outside_change_deletes_the_entry_shown() {
        let hot = |name: &str, path: &str| HotEntry {
            name: name.into(),
            path: path.into(),
        };
        let config = Config {
            hotlist: vec![hot("A", "/a"), hot("B", "/b")],
            ..Config::default()
        };
        let mut app = app_with(config.clone(), State::default());
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::Key(Action::Down)); // on B
        // Edited outside meanwhile: an entry added at the top.
        let edited = Config {
            hotlist: vec![hot("X", "/x"), hot("A", "/a"), hot("B", "/b")],
            ..config
        };
        let _ = app.update(Message::Config(Box::new(edited)));
        let _ = app.update(Message::Key(Action::Delete));
        let left: Vec<&str> = app.config.hotlist.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(left, ["X", "A"]); // B, the one under the cursor, not A
    }

    #[test]
    fn hotlist_hot_letters_pick_or_step() {
        let hot = |name: &str, path: &str| HotEntry {
            name: name.into(),
            path: path.into(),
        };
        let tmp = std::env::temp_dir();
        let config = Config {
            hotlist: vec![
                hot("&Wiki", "/w1"),
                hot("&Temp", &tmp.display().to_string()),
                hot("&Work", "/w2"),
                hot("R&&D", "/rd"),
            ],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let cursor = |app: &App| match &app.dialog {
            Some(Dialog::List { cursor, .. }) => *cursor,
            _ => panic!("hotlist not open"),
        };
        let _ = app.update(Message::Key(Action::Hotlist));
        // Shared hot letter: steps, does not open.
        let _ = app.update(Message::CmdType('W'));
        assert_eq!(cursor(&app), 2);
        let _ = app.update(Message::CmdType('w'));
        assert_eq!(cursor(&app), 0);
        // `&&` is no hot letter: "r" only moves there.
        let _ = app.update(Message::CmdType('r'));
        assert_eq!(cursor(&app), 3);
        // The only one with "t": opened at once.
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::CmdType('t'));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn regression_hot_letters_not_taken_by_back_row_or_dir_names() {
        let hot = |name: &str, path: &str| HotEntry {
            name: name.into(),
            path: path.into(),
        };
        let tmp = std::env::temp_dir();
        let config = Config {
            hotlist: vec![
                hot("-&Projects", ""),
                hot("&Pop", &tmp.display().to_string()),
                hot("--", ""),
            ],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::CmdType('p')); // the only "p" here: into the submenu
        let _ = app.update(Message::CmdType('p')); // "‹ Projects" has no hot letter: Pop opens
        assert!(app.dialog.is_none());
        assert_eq!(crate::hotlist::menu_name(Path::new("/x/R&D")), "R&&D");
    }

    #[test]
    fn hotlist_submenus_open_go_back_and_take_new_dirs() {
        let hot = |name: &str, path: &str| HotEntry {
            name: name.into(),
            path: path.into(),
        };
        let config = Config {
            hotlist: vec![
                hot("a", "/a"),
                hot("-Work", ""),
                hot("w", "/w"),
                hot("--", ""),
            ],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let labels = |app: &App| match &app.dialog {
            Some(Dialog::List { items, .. }) => {
                items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
            }
            _ => panic!("hotlist not open"),
        };
        let _ = app.update(Message::Key(Action::Hotlist));
        assert_eq!(labels(&app)[..2], ["a", "Work"]);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Right)); // into "Work"
        assert_eq!(labels(&app)[..2], ["‹ Work", "w"]);
        assert_eq!(hotlist_items(&app)[1], PathBuf::from("/w"));
        let _ = app.update(Message::ListPick(labels(&app).len() - 2)); // add the panel's dir here
        assert_eq!(
            app.config.hotlist[3].path,
            app.panes[0].active().panel.cwd()
        );
        assert_eq!(app.config.hotlist[4].name, "--");
        let _ = app.update(Message::Key(Action::Hotlist));
        let _ = app.update(Message::ListPick(1));
        let _ = app.update(Message::Key(Action::Parent)); // Backspace: back to the top, on "Work"
        assert_eq!(labels(&app)[1], "Work");
        assert!(matches!(&app.dialog, Some(Dialog::List { cursor: 1, .. })));
    }

    #[test]
    fn the_running_job_pauses_and_a_queued_one_can_be_taken_out() {
        let mut app = files_app(&["a", "b", "c"]);
        let dir = app.panes[0].active().panel.cwd().to_path_buf();
        for n in ["a", "b", "c"] {
            let _ = app.start_job(
                0,
                OpKind::Delete,
                Job::Delete {
                    paths: vec![dir.join(n)],
                    permanent: true,
                },
                None,
            );
        }
        let _ = app.update(Message::JobPause);
        assert!(app.job.as_ref().unwrap().paused());
        let _ = app.update(Message::JobPause);
        assert!(!app.job.as_ref().unwrap().paused());
        let labels = |app: &App| app.queue.iter().map(Queued::label).collect::<Vec<_>>();
        assert_eq!(labels(&app), ["b", "c"]);
        let _ = app.update(Message::QueueUp(1));
        assert_eq!(labels(&app), ["c", "b"]);
        let _ = app.update(Message::QueueUp(0)); // already first
        assert_eq!(labels(&app), ["c", "b"]);
        let _ = app.update(Message::Unqueue(0));
        assert_eq!(labels(&app), ["b"]);
        app.cancel_job();
    }

    #[test]
    fn a_job_in_the_background_frees_the_panels_and_a_second_one_waits_in_the_queue() {
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
        assert!(
            !app.job
                .as_ref()
                .unwrap()
                .control
                .cancel
                .load(Ordering::Relaxed)
        );
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
        assert_eq!(app.msg(), Some(fl!("job-queued", n = 1).as_str()));
        assert_eq!(app.queue.len(), 1);
        let _ = app.update(Message::JobShow);
        assert!(app.busy());
        // The first one ends: the queued one starts, in the background.
        let _ = app.finish_job(&Report::default());
        assert!(app.queue.is_empty());
        assert!(
            app.job
                .as_ref()
                .is_some_and(|j| j.hidden && j.kind == OpKind::Delete)
        );
        assert!(!app.busy());
        // Cancel empties the queue as well.
        let nothing = Job::Delete {
            paths: Vec::new(),
            permanent: true,
        };
        let _ = app.start_job(0, OpKind::Delete, nothing, None);
        assert_eq!(app.queue.len(), 1);
        let _ = app.update(Message::CancelJob);
        assert!(app.queue.is_empty());
    }

    fn idle_job() -> Job {
        Job::Delete {
            paths: Vec::new(),
            permanent: true,
        }
    }

    #[test]
    fn regression_a_queued_copy_is_planned_when_it_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let (f, new) = (tmp.path().join("f"), tmp.path().join("new"));
        std::fs::write(&f, "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
        app.dialog = Some(Dialog::Input {
            op: InputOp::Copy,
            side: 0,
            sources: vec![f],
            input: new.display().to_string(),
        });
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.queue.len(), 1);
        // The running one made the target dir meanwhile: the file goes into it, not over it.
        std::fs::create_dir(&new).unwrap();
        let _ = app.finish_job(&Report::default());
        let copied = new.join("f");
        for _ in 0..300 {
            if copied.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(copied.exists());
    }

    #[test]
    fn regression_every_job_dropped_from_the_queue_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
        for name in ["one", "two"] {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
            app.dialog = Some(Dialog::Input {
                op: InputOp::Copy,
                side: 0,
                sources: vec![tmp.path().join(name)],
                input: tmp.path().join(format!("to-{name}")).display().to_string(),
            });
            let _ = app.update(Message::DialogSubmit);
        }
        assert_eq!(app.queue.len(), 2);
        // Each target became a link to its own source meanwhile: a dir into itself, refused.
        for name in ["one", "two"] {
            std::os::unix::fs::symlink(name, tmp.path().join(format!("to-{name}"))).unwrap();
        }
        let _ = app.finish_job(&Report::default());
        assert!(app.job.is_none() && app.queue.is_empty());
        let msg = app.msg().unwrap();
        assert!(msg.contains("one") && msg.contains("two"), "{msg}");
    }

    #[test]
    fn regression_quick_search_up_in_an_empty_list() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0]
            .active_mut()
            .panel
            .set_listing("/".into(), Vec::new(), None);
        app.search = Some(Search {
            side: 0,
            text: "a".into(),
            filter: false,
        });
        let _ = app.update(Message::Key(Action::Up)); // was: `0 + 0 - 1`, overflow
        assert_eq!(app.panes[0].active().panel.cursor(), 0);
    }

    #[test]
    fn regression_closing_the_window_asks_while_a_job_runs_or_waits() {
        let mut app = files_app(&[]);
        assert!(matches!(app.on_app_exit(), Some(Message::Quit)));
        let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
        let _ = app.update(Message::Quit);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmExit)));
        let _ = app.update(Message::DialogCancel); // stay: the job goes on
        assert!(app.dialog.is_none());
        assert!(
            !app.job
                .as_ref()
                .unwrap()
                .control
                .cancel
                .load(Ordering::Relaxed)
        );
        // A job's question steps aside for the exit dialog and comes back after Cancel.
        let (reply, _rx) = std::sync::mpsc::channel();
        let _ = app.on_job_event(jobs::Event::Error {
            path: "x".into(),
            error: "e".into(),
            reply,
        });
        let _ = app.update(Message::Quit);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmExit)));
        let _ = app.update(Message::DialogCancel);
        assert!(matches!(app.dialog, Some(Dialog::Error { .. })));
        let _ = app.update(Message::Quit);
        let _ = app.update(Message::DialogSubmit);
        assert!(app.held.is_none() && app.quitting);
        assert!(
            app.job
                .as_ref()
                .unwrap()
                .control
                .cancel
                .load(Ordering::Relaxed)
        );
    }

    #[test]
    fn regression_a_background_question_waits_for_the_open_dialog() {
        let mut app = files_app(&["a"]);
        let _ = app.update(Message::Key(Action::Mkdir));
        let (reply, _rx) = std::sync::mpsc::channel();
        let _ = app.on_job_event(jobs::Event::Error {
            path: "x".into(),
            error: "e".into(),
            reply,
        });
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
        let _ = app.update(Message::DialogCancel);
        assert!(matches!(app.dialog, Some(Dialog::Error { .. })));
    }

    #[test]
    fn f2_in_the_copy_dialog_runs_it_in_the_background() {
        let (_tmp, mut app, a) = paste_setup();
        let _ = app.update(Message::Drop {
            side: 0,
            dir: None,
            paths: vec![a],
            mv: false,
        });
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Copy,
                ..
            })
        ));
        let _ = app.update(Message::FieldKey(Action::Rename)); // F2
        assert!(app.dialog.is_none());
        assert!(app.job.as_ref().is_some_and(|j| j.hidden));
    }

    #[test]
    fn dragging_a_header_edge_resizes_and_is_saved_on_release() {
        let mut app = app_with(Config::default(), State::default());
        let before = app.config.columns;
        let _ = app.update(Message::ColDragStart(0, 3)); // Size | Date
        let _ = app.update(Message::ColDrag(500.0)); // first move: where the drag starts
        let _ = app.update(Message::ColDrag(530.0));
        assert_eq!(app.cols[1], before[1] + 30.0); // Size wider, shown at once
        assert_eq!(app.config.columns, before); // not saved yet
        let _ = app.update(Message::ColDragEnd);
        assert_eq!(app.config.columns[1], before[1] + 30.0);
        assert!(app.col_drag.is_none());
    }

    #[test]
    fn reset_columns_restores_and_saves_the_defaults() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::ColDragStart(0, 2));
        let _ = app.update(Message::ColDrag(0.0));
        let _ = app.update(Message::ColDrag(40.0));
        let _ = app.update(Message::ColDragEnd);
        assert_ne!(app.config.columns, Config::default().columns);
        let _ = app.update(Message::Key(Action::ResetColumns));
        assert_eq!(app.cols, Config::default().columns);
        assert_eq!(app.config.columns, Config::default().columns);
    }

    #[test]
    fn regression_column_drag_ends_on_any_release_and_esc_undoes_it() {
        // The release over the file list never reached the pane's own handler: the drag stuck.
        let up = cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
            cosmic::iced::mouse::Button::Left,
        ));
        let id = cosmic::iced::window::Id::unique();
        assert!(matches!(
            route_event(up, event::Status::Captured, id),
            Some(Message::ColDragEnd)
        ));
        let mut app = app_with(Config::default(), State::default());
        let before = app.cols;
        let _ = app.update(Message::ColDragStart(0, 2));
        let _ = app.update(Message::ColDrag(100.0));
        let _ = app.update(Message::ColDrag(150.0));
        assert_ne!(app.cols, before);
        let _ = app.update(Message::Escape(app.window_id()));
        assert!(app.col_drag.is_none());
        assert_eq!(app.cols, before);
    }

    #[test]
    fn shift_f4_creates_the_file_and_puts_the_cursor_on_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("old.txt"), "kept").unwrap();
        // `true` as the editor: nothing opens on the desktop running the tests.
        let config = Config {
            editor: vec!["true".into()],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // old.txt
        let _ = app.update(Message::Key(Action::NewFile));
        assert!(
            matches!(&app.dialog, Some(Dialog::Input { op: InputOp::NewFile, input, .. }) if input == "old.txt")
        );
        let _ = app.update(Message::DialogSubmit); // exists: opened, not emptied
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("old.txt")).unwrap(),
            "kept"
        );
        let _ = app.update(Message::Key(Action::NewFile));
        let _ = app.update(Message::DialogInput("new.md".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(tmp.path().join("new.md").is_file());
        let _ = app.update(Message::Key(Action::NewFile));
        let _ = app.update(Message::DialogInput("a/b".into())); // one name only
        let _ = app.update(Message::DialogSubmit);
        assert!(!tmp.path().join("a").exists());
    }

    #[test]
    fn history_clear_and_keep_history_off() {
        let mut app = app_with(Config::default(), State::default());
        for p in ["/a", "/b"] {
            app.panes[0].active_mut().history.visit(Path::new(p));
        }
        let _ = app.update(Message::Key(Action::HistoryList));
        let _ = app.update(Message::ListClear);
        assert!(app.dialog.is_none());
        assert_eq!(
            app.panes[0].active().history.recent(),
            [PathBuf::from("/b")]
        );
        app.commands = vec!["ls".into()];
        let _ = app.update(Message::Key(Action::CmdHistory));
        let _ = app.update(Message::ListClear);
        assert!(app.commands.is_empty());
        // Off: kept for this run, not written.
        app.commands = vec!["ls".into()];
        app.masks = vec!["*.rs".into()];
        let _ = app.update(Message::Setting(crate::drawer::Setting::KeepHistory(false)));
        assert_eq!(app.commands, ["ls"]);
        assert!(app.saved.commands.is_empty() && app.saved.masks.is_empty());
        assert!(app.saved.panes[0].history.is_empty());
        let _ = app.update(Message::Setting(crate::drawer::Setting::KeepHistory(true)));
        assert_eq!(app.saved.commands, ["ls"]);
    }

    #[test]
    fn regression_dir_history_survives_a_restart() {
        let mut app = app_with(Config::default(), State::default());
        for p in ["/a", "/b"] {
            app.panes[0].active_mut().history.visit(Path::new(p));
        }
        let _ = app.update(Message::Key(Action::HistoryList)); // any message saves the state
        let _ = app.update(Message::DialogCancel);
        let saved = app.saved.clone();
        assert_eq!(saved.panes[0].history.len(), saved.panes[0].tabs.len());
        let again = app_with(Config::default(), saved);
        let recent = again.panes[0].active().history.recent();
        assert!(recent.contains(&PathBuf::from("/a")), "{recent:?}");
    }

    #[test]
    fn regression_ctrl_is_let_go_when_the_window_loses_focus() {
        let ev = cosmic::iced::Event::Window(cosmic::iced::window::Event::Unfocused);
        let id = cosmic::iced::window::Id::unique();
        let m = route_event(ev, event::Status::Ignored, id);
        assert!(matches!(m, Some(Message::Modifiers(m)) if m.is_empty()));
    }

    #[test]
    fn the_terminal_is_chosen_in_the_menu_and_runs_shift_enter() {
        let mut app = app_with(Config::default(), State::default());
        let term = |id: &str, prog: &str| Terminal {
            id: id.into(),
            name: id.into(),
            argv: vec![prog.into()],
        };
        let list = vec![term("a.desktop", "alacritty"), term("f.desktop", "foot")];
        let _ = app.update(Message::Terminals(list, Some(0)));
        assert_eq!(app.terminal_prefix(), ["alacritty", "-e"]); // the system's default
        let pick = |app: &mut App, row: usize| {
            let _ = app.update(Message::Key(Action::PickTerminal));
            let _ = app.update(Message::ListPick(row));
        };
        pick(&mut app, 2); // .., default, a, f → "f"
        assert_eq!(app.config.terminal_app, "f.desktop");
        assert_eq!(app.terminal_prefix(), ["foot"]);
        let _ = app.update(Message::Key(Action::PickTerminal));
        match &app.dialog {
            Some(Dialog::List { cursor, items, .. }) => {
                assert_eq!(*cursor, 2); // the chosen one
                assert!(items[2].label.starts_with('✓') && !items[0].label.starts_with('✓'));
            }
            _ => panic!("no list"),
        }
        app.dialog = None;
        pick(&mut app, 0);
        assert_eq!(app.config.terminal_app, "");
        // `terminal` changed by hand (before the menu existed): it still wins.
        app.config.terminal = vec!["xterm".into(), "-e".into()];
        assert_eq!(app.terminal_prefix(), ["xterm", "-e"]);
    }

    #[test]
    fn columns_grow_with_the_text_and_are_kept_for_the_default_size() {
        let mut app = app_with(Config::default(), State::default());
        let base = app.cols;
        app.config.panel_size = 20;
        let k = app.config.text_scale();
        assert!(k > 1.0);
        assert_eq!(app.col_widths(), base.map(|w| w * k));
        // A drag of 20 px on screen at the bigger size: 20 / k kept.
        let _ = app.update(Message::ColDragStart(0, 3));
        let _ = app.update(Message::ColDrag(100.0));
        let _ = app.update(Message::ColDrag(120.0));
        assert!((app.cols[1] - (base[1] + 20.0 / k)).abs() < 0.01);
    }

    #[test]
    fn found_in_archives_are_listed_and_deleted_one_archive_at_a_time() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let zip_at = |name: &str| {
            let a = tmp.path().join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
            z.start_file("f.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
            a
        };
        let (a, b) = (zip_at("a.zip"), zip_at("b.zip"));
        let disk = tmp.path().join("d.txt");
        std::fs::write(&disk, "d").unwrap();
        let found = vec![disk.clone(), a.join("f.txt"), b.join("f.txt")];
        let rows = read_listing(tmp.path(), false, Some(&found), &AtomicBool::new(false)).unwrap();
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["d.txt", "a.zip/f.txt", "b.zip/f.txt"]);
        assert_eq!(
            located(tmp.path(), &rows[1].os_name),
            (a.clone(), "f.txt".into())
        );
        // F8 on all three: a's rewrite runs, b's and the disk file's wait their turn (never run
        // here: the trash is not the test's to touch).
        let mut app = app_with(Config::default(), State::default());
        let _ = app.start_delete(0, false, vec![a.join("f.txt"), b.join("f.txt"), disk]);
        assert!(app.job.is_some());
        assert_eq!(app.queue.len(), 2);
    }

    #[test]
    fn a_nested_archive_shows_where_it_lies_and_is_never_changed() {
        let mut app = app_with(Config::default(), State::default());
        let tmp = archive::temp_root().join("1/0/inner.zip");
        app.nested
            .insert(tmp.clone(), PathBuf::from("/d/outer.zip/sub"));
        let shown = PathBuf::from("/d/outer.zip/sub/inner.zip/x");
        assert_eq!(app.shown(&tmp.join("x")), shown);
        assert_eq!(app.real(&shown), tmp.join("x"));
        assert_eq!(
            app.real(Path::new("/d/outer.zip")),
            PathBuf::from("/d/outer.zip")
        );
        // One deeper: an archive opened from inside the nested one.
        let deeper = archive::temp_root().join("1/1/core.7z");
        app.nested.insert(deeper.clone(), tmp.join("lib"));
        let far = PathBuf::from("/d/outer.zip/sub/inner.zip/lib/core.7z");
        assert_eq!(app.shown(&deeper), far);
        assert_eq!(app.real(&far.join("a")), deeper.join("a"));
        let change = shagoff_core::repack::Change::Delete(vec![PathBuf::from("x")]);
        let repack = |archive: PathBuf| Job::Repack {
            archive,
            change: change.clone(),
        };
        assert!(changes_nested(&repack(tmp)));
        assert!(!changes_nested(&repack("/d/outer.zip".into())));
    }

    #[test]
    fn shift_enter_from_the_panel_runs_the_line_else_enters_a_dir() {
        let mut app = app_with(Config::default(), State::default());
        app.config.terminal = vec!["true".into()]; // a harmless "terminal"
        app.cmdline = "echo hi".into(); // typed, then a click on the panel
        let _ = app.update(Message::Key(Action::RunTerminal));
        assert!(app.cmdline.is_empty());
        assert_eq!(app.commands.first().map(String::as_str), Some("echo hi"));
        // Nothing typed, the cursor on a dir: Shift+Enter enters it, as Enter.
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
            focus: Some("sub".into()),
            result: Ok(vec![sub]),
            space: None,
        });
        let _ = app.update(Message::Key(Action::RunTerminal));
        assert_eq!(pending_path(&app), Some(cwd.join("sub")));
    }

    #[test]
    fn builtin_terminal_rows_shell_and_ctrl_o() {
        let mut app = app_with(Config::default(), State::default());
        // Off: Ctrl+O does nothing (TC's console is not ours to hide panels for).
        let _ = app.update(Message::Key(Action::TermFull));
        assert!(!app.term_full);
        let _ = app.update(Message::Key(Action::ToggleTerminal));
        assert!(app.config.term_builtin);
        // Its top edge dragged up 3 lines: 3 rows more, saved on release.
        let line = (f32::from(app.config.panel_size()) * 1.4).ceil();
        let rows = app.config.term_rows;
        let _ = app.update(Message::TermDragStart);
        let _ = app.update(Message::TermDragAt(400.0));
        let _ = app.update(Message::TermDragAt(400.0 - 3.0 * line));
        assert_eq!(app.term_rows(), rows + 3);
        assert_eq!(app.config.term_rows, rows); // not yet
        let _ = app.update(Message::TermDragEnd);
        assert_eq!(app.config.term_rows, rows + 3);
        // Commands → Shell…: `$SHELL` first, a pick sets it.
        let _ = app.update(Message::Key(Action::PickShell));
        let Some(Dialog::List { items, .. }) = &app.dialog else {
            panic!("no list");
        };
        let last = items.len() - 1;
        let shell = items[last].path.clone();
        let _ = app.update(Message::ListPick(last));
        assert_eq!(PathBuf::from(&app.config.term_shell), shell);
        let _ = app.update(Message::Key(Action::ToggleTerminal));
        assert!(!app.config.term_builtin && app.term.is_none());
    }

    #[test]
    fn regression_keys_the_focused_terminal_took_do_not_act_on_the_panels() {
        let mut app = files_app(&["a"]);
        let _ = app.update(Message::Key(Action::Down)); // on "a"
        let _ = app.update(Message::TermFocused(true));
        // F8 in mc, Ctrl+R in bash: `route_event` passes them on as field keys.
        let _ = app.update(Message::FieldKey(Action::Delete));
        let _ = app.update(Message::FieldKey(Action::Reload));
        assert!(app.dialog.is_none());
        let _ = app.update(Message::TermFocused(false));
        let _ = app.update(Message::FieldKey(Action::Delete));
        assert!(app.dialog.is_some()); // the panels' again
    }

    #[test]
    fn mask_dialog_remembers_masks() {
        let mut app = app_with(Config::default(), State::default());
        let mask = |app: &App| match &app.dialog {
            Some(Dialog::Mask { input, .. }) => input.clone(),
            _ => panic!("no mask dialog"),
        };
        let _ = app.update(Message::Key(Action::SelectGroup));
        assert_eq!(mask(&app), "*.*"); // nothing typed yet: as TC
        app.dialog = None;
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
    fn regression_hotlist_add_of_a_non_utf8_dir_says_why() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
        std::fs::create_dir(&dir).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, &dir);
        let _ = app.update(Message::Key(Action::Hotlist));
        let add = match &app.dialog {
            Some(Dialog::List { items, .. }) => items.iter().position(|i| i.kind == Item::Add),
            _ => None,
        };
        let _ = app.pick(add.expect("«Add»"));
        assert!(app.config.hotlist.is_empty());
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
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
        let _ = app.update(Message::Config(Box::new(Config {
            show_hidden: true,
            ..Config::default()
        })));
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

    #[test]
    fn viewer_plus_minus_change_its_text_size_and_save_it() {
        let (_tmp, mut app, _) = paste_setup();
        app.lister = Some(Box::new(Lister::new(0, String::new(), 0, 14)));
        let _ = app.update(Message::ListerKey(ListerKey::Zoom(true))); // still reading: nothing
        assert_eq!(app.config.lister_size, 14);
        let text = crate::lister::Loaded::read(&_tmp.path().join("src/a"), "a").unwrap();
        app.lister.as_mut().unwrap().set_loaded(Arc::new(Ok(text)));
        let _ = app.update(Message::ListerKey(ListerKey::Zoom(true)));
        let _ = app.update(Message::Key(Action::SelectGroup)); // Num+
        assert_eq!(app.config.lister_size, 16);
        assert_eq!(app.lister.as_ref().map(|l| l.size), Some(16));
        let _ = app.update(Message::Key(Action::UnselectGroup)); // Num-
        assert_eq!(app.config.lister_size, 15);
        // A picture: no text to size.
        app.lister.as_mut().unwrap().mode = shagoff_core::lister::Mode::Image;
        let _ = app.update(Message::ListerKey(ListerKey::Zoom(true)));
        assert_eq!(app.config.lister_size, 15);
    }

    #[test]
    fn regression_closing_the_viewer_scrolls_the_lists_back() {
        // F3 replaces the panels; Esc rebuilds their lists at the top while the tabs keep
        // their offsets: both must be scrolled back (two more task units than without a
        // viewer), or the cursor at the end is out of view.
        let (_tmp, mut app, _) = paste_setup();
        let viewer = |quick: bool| {
            let mut l = Lister::new(0, String::new(), 0, 14);
            l.quick = quick.then(|| (PathBuf::new(), 0, std::time::UNIX_EPOCH));
            Box::new(l)
        };
        let id = app.window_id();
        // (Ctrl+Q?, the message): F3 closed by its button or Esc; Ctrl+Q, Tab moving it.
        let cases = [
            (false, Message::ListerClose),
            (false, Message::Escape(id)),
            (true, Message::Key(Action::SwitchPane)),
        ];
        for (quick, msg) in cases {
            let plain = app.update(msg.clone()).units();
            app.lister = Some(viewer(quick));
            let units = app.update(msg.clone()).units();
            assert_eq!(units, plain + 2, "{msg:?}");
        }
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
    fn drop_opens_the_copy_dialog_aimed_at_the_dir_row_or_the_pane() {
        let (tmp, mut app, a) = paste_setup();
        let dst = tmp.path().join("dst");
        std::fs::create_dir(dst.join("sub")).unwrap();
        listed_at(&mut app, 0, &dst);
        let drop = |dir, mv| Message::Drop {
            side: 0,
            dir,
            paths: vec![a.clone()],
            mv,
        };
        let _ = app.update(drop(Some(dst.join("sub")), false));
        let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
            panic!("no dialog")
        };
        assert_eq!((op, input), (InputOp::Copy, dir_input(&dst.join("sub"))));
        let _ = app.update(drop(None, true)); // Shift: move, into the pane's dir
        let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
            panic!("no dialog")
        };
        assert_eq!((op, input), (InputOp::Move, dir_input(&dst)));
        // Dragged from the other pane: its marks go when the copy is done, as after F5.
        listed_at(&mut app, 1, &tmp.path().join("src"));
        let _ = app.update(drop(None, false));
        assert!(matches!(
            app.dialog.take(),
            Some(Dialog::Input { side: 1, .. })
        ));
        // Shift held while dropping: move, whatever action the compositor reports.
        let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
        let _ = app.update(drop(None, false));
        assert!(matches!(
            app.dialog.take(),
            Some(Dialog::Input {
                op: InputOp::Move,
                ..
            })
        ));
        let _ = app.update(Message::Modifiers(Modifiers::empty()));
        // Back into the dir it came from: nothing.
        listed_at(&mut app, 0, &tmp.path().join("src"));
        let _ = app.update(drop(None, false));
        assert!(app.dialog.is_none());
        assert_eq!(app.drop_hover, None);
    }

    #[test]
    fn colors_by_type_are_edited_on_a_copy_and_saved_on_ok() {
        use crate::colorrules::ColorMsg;
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::ConfigureColors));
        assert!(matches!(app.dialog, Some(Dialog::Colors(_))));
        let _ = app.update(Message::Colors(ColorMsg::Add));
        assert_eq!(app.config.colors, shagoff_core::colors::defaults()); // not yet
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.config.colors.len(), 3);
        let _ = app.update(Message::Key(Action::ConfigureColors));
        let _ = app.update(Message::Colors(ColorMsg::Delete));
        let _ = app.update(Message::DialogCancel); // Cancel: unchanged
        assert_eq!(app.config.colors.len(), 3);
    }

    #[test]
    fn right_button_drop_asks_copy_move_or_link() {
        let (tmp, mut app, a) = paste_setup();
        let dst = tmp.path().join("dst");
        let drop = |paths: Vec<PathBuf>| Message::Drop {
            side: 0,
            dir: None,
            paths,
            mv: false,
        };
        let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
        // The window's own DnD motion (not a zone's, shifted by its scrollable) places the menu.
        let motion = cosmic::iced::Event::Dnd(DndEvent::Offer(
            None,
            OfferEvent::Motion { x: 100.0, y: 50.0 },
        ));
        let at = route_event(motion, event::Status::Captured, app.window_id());
        let _ = app.update(at.expect("motion is routed"));
        let _ = app.update(drop(vec![a.clone()]));
        let Some(Dialog::DropMenu { at, .. }) = &app.dialog else {
            panic!("no drop menu")
        };
        assert_eq!(*at, cosmic::iced::Point::new(100.0, 50.0));
        // ↑ from Copy wraps to Cancel; ↓ ↓ goes Copy → Move; Enter picks it.
        let _ = app.update(Message::Key(Action::Up));
        assert!(matches!(
            app.dialog,
            Some(Dialog::DropMenu { cursor: 3, .. })
        ));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Enter));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Move,
                ..
            })
        ));
        // A hot letter picks at once (en labels in tests: &Copy &Move &Link).
        app.dialog = None;
        let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
        let _ = app.update(drop(vec![a.clone()]));
        let _ = app.update(Message::CmdType('L'));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Link,
                ..
            })
        ));
        app.dialog = None;
        let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
        let _ = app.update(drop(vec![a.clone()]));
        let _ = app.update(Message::DropAs(InputOp::Link));
        let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
            panic!("no dialog")
        };
        assert_eq!((op, input), (InputOp::Link, dir_input(&dst)));
        // Used once; a left drag (or files from another program) gets the copy dialog.
        let _ = app.update(drop(vec![a.clone()]));
        assert!(matches!(
            app.dialog.take(),
            Some(Dialog::Input {
                op: InputOp::Copy,
                ..
            })
        ));
        let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
        let other = tmp.path().join("src/other");
        let _ = app.update(drop(vec![other]));
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    }

    #[test]
    fn drop_on_a_tab_or_drive_button_works_beside_search_results() {
        let (tmp, mut app, a) = paste_setup();
        let t = app.panes[0].active_mut();
        t.results = Some((tmp.path().to_path_buf(), Arc::new(Vec::new())));
        let drop = |dir: Option<PathBuf>| Message::Drop {
            side: 0,
            dir,
            paths: vec![a.clone()],
            mv: false,
        };
        let _ = app.update(drop(None)); // the results list itself: no one dir
        assert!(app.dialog.is_none());
        let other = tmp.path().join("dst");
        let _ = app.update(drop(Some(other.clone()))); // another tab's dir / a drive
        let Some(Dialog::Input { input, .. }) = app.dialog.take() else {
            panic!("no dialog")
        };
        assert_eq!(input, dir_input(&other));
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
    fn mr_token_buttons_append_and_help_toggles() {
        let (_tmp, mut app) = mr_setup();
        let _ = app.update(Message::MrInsert("_[C]"));
        assert_eq!(mr(&app).rows(&TimeZone::UTC)[1].new, "b_2.txt");
        assert!(!mr(&app).help);
        let _ = app.update(Message::MrHelp);
        assert!(mr(&app).help);
        let _ = app.update(Message::MrHelp);
        assert!(!mr(&app).help);
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
    fn pack_password_must_be_typed_twice_alike() {
        let (_tmp, mut app) = pack_setup();
        let _ = app.update(Message::Key(Action::Pack));
        let _ = app.update(Message::PackFormat(Format::Zip));
        let _ = app.update(Message::Toggle(dialogs::Toggle::Encrypt));
        // «Шифровать» with no password: OK does nothing.
        let _ = app.update(Message::DialogSubmit);
        assert!(app.job.is_none() && matches!(app.dialog, Some(Dialog::Pack(_))));
        let _ = app.update(Message::PackPassword(false, "pw".into()));
        let _ = app.update(Message::PackPassword(true, "pq".into()));
        let _ = app.update(Message::DialogSubmit);
        assert!(app.job.is_none() && matches!(app.dialog, Some(Dialog::Pack(_))));
        let _ = app.update(Message::PackPassword(true, "pw".into()));
        let Some(Dialog::Pack(p)) = &app.dialog else {
            panic!()
        };
        assert_eq!(p.password(), Ok(Some("pw".into())));
        // tar: no encryption, the fields are not used.
        let _ = app.update(Message::PackFormat(Format::TarGz));
        let Some(Dialog::Pack(p)) = &app.dialog else {
            panic!()
        };
        assert_eq!(p.password(), Ok(None));
        let _ = app.update(Message::DialogSubmit);
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
    fn alt_enter_inside_an_archive_shows_and_changes_nothing() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::End)); // top.txt
        let _ = app.update(Message::Key(Action::Properties));
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no properties dialog");
        };
        assert_eq!(p.archive.as_deref(), Some(a.as_path()));
        assert_eq!(p.paths, [a.join("top.txt")]);
        assert!(p.facts.iter().any(|(_, v)| v == "top.txt"));
        let mode = p.mode;
        let _ = app.update(Message::PropsBit(0o100));
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("closed");
        };
        assert_eq!((p.mode, p.touched), (mode, 0));
        let _ = app.update(Message::DialogSubmit); // Enter: just closes
        assert!(app.dialog.is_none());
    }

    #[test]
    fn ctrl_b_inside_an_archive_lists_from_its_index() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let tab = app.panes[0].active().id;
        let _ = app.update(Message::Key(Action::Branch));
        let id = app.panes[0]
            .active()
            .branching
            .expect("not refused inside an archive");
        let paths = shagoff_core::archive::branch(&a, Path::new(""), false).ok();
        let _ = app.update(Message::BranchListed {
            tab,
            id,
            root: a.clone(),
            paths,
        });
        let (root, paths) = app.panes[0].active().results.clone().unwrap();
        let mut names: Vec<String> =
            read_listing(&root, false, Some(&paths), &AtomicBool::new(false))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
        names.sort();
        assert_eq!(names, ["d/f.txt", "top.txt"]);
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
    fn ctrl_m_inside_archive_repacks() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::End));
        let _ = app.update(Message::Key(Action::MultiRename));
        let Some(Dialog::MultiRename(m)) = &mut app.dialog else {
            panic!("{:?}", app.msg())
        };
        m.rule.name = "x[N]".into();
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
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
    fn regression_edited_file_is_asked_again_when_putting_it_back_failed() {
        let (tmp, mut app, a) = zip_setup();
        let file = tmp.path().join("out/top.txt");
        std::fs::write(&file, "x").unwrap();
        app.edited.push(Edited {
            file,
            archive: a,
            entry: "top.txt".into(),
            mtime: std::time::UNIX_EPOCH,
        });
        let _ = app.update(Message::EditTick);
        let _ = app.update(Message::DialogSubmit); // «Обновить»
        let failed = Arc::new(Report::default()); // nothing completed: an error
        let _ = app.update(Message::Op(jobs::Event::Finished(failed)));
        let _ = app.update(Message::EditTick);
        assert!(matches!(app.dialog, Some(Dialog::UpdateArchive { .. })));
        // Put back this time: not asked again.
        let _ = app.update(Message::DialogSubmit);
        let done = Arc::new(Report {
            cancelled: false,
            completed: vec![PathBuf::from("a.zip")],
        });
        let _ = app.update(Message::Op(jobs::Event::Finished(done)));
        let _ = app.update(Message::EditTick);
        assert!(app.dialog.is_none());
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
    fn drops_out_of_and_into_an_archive_work_as_f5() {
        let (tmp, mut app, a) = zip_setup();
        let drop = |side, paths, dir: Option<PathBuf>, mv| Message::Drop {
            side,
            dir,
            paths,
            mv,
        };
        // From the archive (pane 0 inside it) onto pane 1's list: extracted there.
        listed_at(&mut app, 0, &a);
        let _ = app.update(drop(1, vec![a.join("top.txt")], None, false));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Copy,
                ..
            })
        ));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Extract));
        app.job = None;
        // From disk onto the archive's list: packed in.
        let x = tmp.path().join("out/x");
        std::fs::write(&x, "1").unwrap();
        let _ = app.update(drop(0, vec![x.clone()], None, false));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
        app.job = None;
        // Onto the archive's row in a list (its path as the dir): packed in too.
        let _ = app.update(drop(1, vec![x], Some(a.clone()), false));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
        app.job = None;
        // From the archive into a dir of it: the F5 dialog, then a cross-archive job.
        let _ = app.update(drop(0, vec![a.join("top.txt")], Some(a.join("d")), false));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
        app.job = None;
        // A right-button drop from inside it: no Link in the menu (no file on disk to link to).
        assert_eq!(
            dialogs::drop_ops(&[a.join("top.txt")]),
            [InputOp::Copy, InputOp::Move]
        );
        assert_eq!(dialogs::drop_ops(&[tmp.path().join("out/x")]).len(), 3);
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
    fn clipboard_out_of_and_into_an_archive_works_as_f5() {
        let (tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 0, &a);
        let _ = app.update(Message::Key(Action::Down));
        // Ctrl+C / Ctrl+X inside only put the paths; no job, not read-only.
        let _ = app.update(Message::Key(Action::ClipCut));
        assert!(app.job.is_none() && app.status.is_none());
        // Pasted outside: a cut moves out of the archive (extract, then delete there).
        app.active = 1;
        let _ = app.update(Message::Pasted(Some((
            ClipKind::Cut,
            vec![a.join("top.txt")],
        ))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
        app.job = None;
        // Pasted inside: packed into it.
        app.active = 0;
        let f = tmp.path().join("out/n.txt");
        std::fs::write(&f, "1").unwrap();
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![f]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
        app.job = None;
        // From an archive into the same dir of it: the same file, as on disk.
        let top = a.join("top.txt");
        let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![top.clone()]))));
        assert!(app.job.is_none());
        let same = plan_error(&PlanError::SameFile(top.clone()));
        assert_eq!(app.msg(), Some(same.as_str()));
        // Into another dir of it: a cross-archive job.
        listed_at(&mut app, 0, &a.join("d"));
        let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![top]))));
        assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
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
        let msg = Message::DirSize(id, tmp.path().into(), "sub".into(), 4096);
        let _ = app.update(msg);
        let p = &app.panes[0].active().panel;
        assert_eq!(p.marked_totals().bytes, 4096);
        assert_eq!(p.dir_size(&p.entries()[1]), Some(4096));
    }

    #[test]
    fn regression_a_size_counted_lands_in_a_tab_moved_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let id = app.panes[0].active().id;
        let _ = app.update(Message::Key(Action::SwapPanes)); // Ctrl+U while it counts
        assert_eq!(app.side_of(id), Some(1));
        let _ = app.update(Message::DirSize(id, tmp.path().into(), "sub".into(), 4096));
        let p = &app.panes[1].active().panel;
        let sub = p.entries().iter().find(|e| e.name == "sub").unwrap();
        assert_eq!(p.dir_size(sub), Some(4096));
    }

    #[test]
    fn leaving_the_dir_stops_its_size_counts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let stop = app.panes[0].active().counting.clone();
        listed_at(&mut app, 0, tmp.path()); // a rescan keeps counting
        assert!(!stop.load(Ordering::Relaxed));
        listed_at(&mut app, 0, &tmp.path().join("sub"));
        assert!(stop.load(Ordering::Relaxed));
        assert!(!app.panes[0].active().counting.load(Ordering::Relaxed));
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
    fn divider_drag_moves_the_split_and_is_kept() {
        let mut app = app_with(Config::default(), State::default());
        for side in 0..2 {
            let _ = app.update(Message::Resized(side, Size::new(500.0, 300.0)));
        }
        let w = app.window_id();
        let _ = app.update(Message::SplitAt(w, 300.0)); // not held: nothing
        assert_eq!(app.split, 0.0);
        let _ = app.update(Message::WindowWidth(w, 1004.0)); // 1000 + the 4 px gap
        let _ = app.update(Message::SplitStart);
        let _ = app.update(Message::SplitAt(w, 500.0));
        let _ = app.update(Message::SplitAt(w, 501.0)); // a click's jitter: not moved
        assert_eq!(app.split, 0.0);
        let _ = app.update(Message::SplitAt(w, 302.0)); // Classic: left edge + 2 px of the gap
        assert_eq!(app.saved.split, 0.0); // not written while held
        let _ = app.update(Message::SplitEnd);
        assert!((app.split - 0.3).abs() < 1e-4, "{}", app.split);
        let _ = app.update(Message::SplitAt(w, 900.0)); // let go: no more moves
        assert!((app.saved.split - 0.3).abs() < 1e-4); // written to the state
        let _ = app.update(Message::SplitReset);
        assert_eq!(app.split, 0.5);
    }

    #[test]
    fn sync_options_are_remembered_after_a_compare() {
        use shagoff_core::sync::Kind;
        let (_tmp, mut app) = results_app();
        let _ = app.update(Message::Key(Action::SyncDirs));
        let _ = app.update(Message::SyncOpt(SyncOpt::Content));
        let _ = app.update(Message::SyncOpt(SyncOpt::Recursive)); // off
        let _ = app.update(Message::SyncOpt(SyncOpt::Mirror)); // not kept
        let _ = app.update(Message::SyncCompare);
        // After the compare, not compared again: still kept.
        let _ = app.update(Message::SyncShow(Kind::Same)); // shown now
        let _ = app.update(Message::SyncShow(Kind::ToLeft)); // hidden
        let _ = app.update(Message::SyncMask("*.rs".into()));
        let _ = app.update(Message::DialogCancel);
        assert_eq!(app.saved.sync.mask, "*.rs"); // written to the state
        let mut app = app_with(Config::default(), app.saved.clone());
        let _ = app.update(Message::Key(Action::SyncDirs));
        let Some(Dialog::Sync(s)) = &app.dialog else {
            panic!("no dialog");
        };
        assert!(s.content && !s.recursive && !s.mirror && s.mask == "*.rs");
        assert_eq!(s.hide, HashSet::from([Kind::ToLeft]));
    }

    #[test]
    fn regression_a_bad_regex_clears_the_last_results() {
        let (_tmp, mut app) = results_app();
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindNameRegex);
        let _ = app.update(Message::FindInput(FindField::Mask, "(".into()));
        if let Some(Dialog::Find(f)) = &mut app.dialog {
            f.results.push(PathBuf::from("/a search before"));
        }
        let _ = app.update(Message::FindStart);
        let Some(Dialog::Find(f)) = &app.dialog else {
            panic!("no dialog");
        };
        assert!(f.error.is_some() && f.results.is_empty());
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
    fn alt_enter_changes_the_group_to_one_of_the_users() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a");
        std::fs::write(&f, "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Properties));
        let Some(Dialog::Props(p)) = &app.dialog else {
            panic!("no dialog");
        };
        let now = std::fs::metadata(&f).unwrap().gid();
        assert_eq!(p.group.map(|i| p.gids[i]), Some(now)); // the file's own shown
        let mine = shagoff_core::owners::my_groups();
        let other = (p.gids.iter()).position(|g| *g != now && mine.contains(g));
        let Some(i) = other else { return }; // a user with one group: nothing to pick
        let g = p.gids[i];
        assert!(p.new_group().is_none() && p.new_user().is_none());
        let me = std::fs::metadata(&f).unwrap().uid();
        assert_eq!(p.user.map(|i| p.uids[i]), Some(me)); // the file's owner shown
        let back = p.group;
        let _ = app.update(Message::PropsGroup(i));
        let Some(Dialog::Props(p)) = &mut app.dialog else {
            panic!("no dialog");
        };
        assert_eq!(p.new_group(), Some(g)); // what OK gives chgrp
        p.group = back; // picked the files' own again: nothing to do
        assert_eq!(p.new_group(), None);
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
        assert_eq!(props(&app).1, 0o7777); // setuid, setgid, sticky too
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
        app.config.right_marks = false; // the menu at once
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
    fn right_drag_marks_the_rows_passed_as_tc() {
        let mut app = files_app(&["a", "b", "c", "d", "e"]); // .., a, b, c, d, e
        let names = |app: &App| -> Vec<String> {
            let p = &app.panes[0].active().panel;
            (p.entries().iter())
                .filter(|e| p.is_marked(e))
                .map(|e| e.name.clone())
                .collect()
        };
        let _ = app.update(Message::RightClick(0, 1)); // on "a"
        let _ = app.update(Message::RowEnter(0, 1)); // still on it: nothing yet
        assert!(names(&app).is_empty());
        let _ = app.update(Message::RowEnter(0, 2));
        let _ = app.update(Message::RowEnter(0, 4)); // a quick move skips "c": marked too
        assert_eq!(names(&app), ["a", "b", "c", "d"]);
        assert_eq!(cursor_name(&app), "d");
        let n = app.right_hold.unwrap().n;
        let _ = app.update(Message::RightHeld(n)); // held long while dragging: no menu
        assert!(!app.right_hold.unwrap().armed);
        let _ = app.update(Message::RightUp); // no toggle on release
        assert_eq!(names(&app), ["a", "b", "c", "d"]);
        // From a marked row: the drag unmarks.
        let _ = app.update(Message::RightClick(0, 3));
        let _ = app.update(Message::RowEnter(0, 2));
        let _ = app.update(Message::RightUp);
        assert_eq!(names(&app), ["a", "d"]);
        // Into the other pane: not this drag's rows.
        let _ = app.update(Message::RightClick(0, 5));
        let _ = app.update(Message::RowEnter(1, 0));
        let _ = app.update(Message::RightUp);
        assert_eq!(names(&app), ["a", "d", "e"]); // a plain right click on "e"
    }

    #[test]
    fn right_button_marks_and_a_long_press_opens_the_menu() {
        let mut app = files_app(&["a", "b", "c"]);
        let marked = |app: &App| app.panes[0].active().panel.marked_totals().files;
        let held = |app: &App| app.right_hold.map(|h| h.n).unwrap();
        // quick click: the cursor goes there and the row is marked; again: unmarked
        let _ = app.update(Message::RightClick(0, 1));
        let _ = app.update(Message::RightEmpty(0)); // the list's own handler, same press
        let _ = app.update(Message::RightUp);
        assert_eq!((cursor_name(&app), marked(&app)), ("a".into(), 1));
        let _ = app.update(Message::RightClick(0, 3));
        let _ = app.update(Message::RightEmpty(0));
        let _ = app.update(Message::RightUp);
        assert_eq!(marked(&app), 2);
        // long press on the unmarked "b": the menu is about it, marks dropped, none added
        let _ = app.update(Message::RightClick(0, 2));
        let _ = app.update(Message::RightEmpty(0));
        let stale = held(&app) - 1;
        let _ = app.update(Message::RightHeld(stale)); // an earlier press's timer: ignored
        assert!(!app.right_hold.unwrap().armed);
        let _ = app.update(Message::RightHeld(held(&app)));
        assert!(app.ctx_entry && app.right_hold.unwrap().armed);
        assert_eq!(marked(&app), 2); // the marks go with the release, when the menu opens
        let _ = app.update(Message::RightUp);
        assert_eq!((marked(&app), app.right_hold), (0, None));
        // held, then dragged: the marks stay
        let _ = app.update(Message::Key(Action::SelectAll));
        let _ = app.update(Message::RightClick(0, 1));
        let _ = app.update(Message::RightEmpty(0));
        let _ = app.update(Message::RightHeld(held(&app)));
        let _ = app.update(Message::DragStart(None));
        let _ = app.update(Message::RightUp);
        assert_eq!(marked(&app), 3);
        let _ = app.update(Message::Key(Action::UnselectAll));
        // pressed, then the cursor moved by a key: the release marks nothing
        let _ = app.update(Message::RightClick(0, 1));
        let _ = app.update(Message::RightEmpty(0));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::RightUp);
        assert_eq!(marked(&app), 0);
        // a right drag: no mark
        let _ = app.update(Message::RightClick(0, 1));
        let _ = app.update(Message::RightEmpty(0));
        let _ = app.update(Message::DragStart(None));
        let _ = app.update(Message::RightUp);
        assert_eq!(marked(&app), 0);
        // empty space: a quick click does nothing, a long one the dir's menu
        let _ = app.update(Message::RightEmpty(1));
        assert_eq!(app.active, 1);
        let _ = app.update(Message::RightHeld(held(&app)));
        assert!(!app.ctx_entry);
        let _ = app.update(Message::RightUp);
        assert_eq!(marked(&app), 0);
    }

    #[test]
    fn regression_menu_open_opens_even_with_a_typed_command() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let config = Config {
            right_marks: false,
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
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
    fn alt_l_feeds_find_results_in_any_layout() {
        let alt_l = |c: &str, status| {
            route_event(
                cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Character(c.into()),
                    modified_key: Key::Character(c.into()),
                    physical_key: Physical::Code(Code::KeyL),
                    location: Location::Standard,
                    modifiers: Modifiers::ALT,
                    text: None,
                    repeat: false,
                }),
                status,
                cosmic::iced::window::Id::unique(),
            )
        };
        // Russian layout too; in a field nothing else would go, in the panel quick search would.
        let m = alt_l("д", event::Status::Captured);
        assert!(matches!(m, Some(Message::FeedKey(None))), "{m:?}");
        let m = alt_l("l", event::Status::Ignored);
        assert!(
            matches!(m, Some(Message::FeedKey(Some(Action::QuickSearch('l'))))),
            "{m:?}"
        );
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.rs");
        std::fs::write(&a, "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::FindFiles));
        let _ = app.update(Message::FindStart);
        let _ = app.update(Message::FeedKey(None)); // nothing found yet
        let id = find_dialog(&mut app).id;
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, vec![a])));
        let mask = app.find.mask.clone();
        // The focused field types the letter too: harmless, the fields were kept at the start.
        let _ = app.update(Message::FindInput(
            dialogs::FindField::Mask,
            format!("{mask}l"),
        ));
        let _ = app.update(Message::FeedKey(None));
        assert!(app.dialog.is_none());
        assert_eq!(app.find.mask, mask);
        assert!(app.panes[0].active().results.is_some());
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
        let e = read_listing(&root, false, Some(&paths), &AtomicBool::new(false)).unwrap();
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
            result: Ok(listing::entries(&[a], &root, &AtomicBool::new(false))),
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
        // An attribute letter that is none of h r s x: said, not searched.
        let _ = app.update(Message::FindInput(FindField::Attrs, "hq".into()));
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.stop.is_none() && f.error.is_some());
        let _ = app.update(Message::FindInput(FindField::Attrs, "x".into()));
        let _ = app.update(Message::FindInput(FindField::Text, "(".into()));
        let _ = app.update(Message::FindRegex);
        // «Искать текст» off: the field is not used, names only — no error.
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.error.is_none());
        let _ = app.update(Message::FindTextOn);
        let _ = app.update(Message::FindStart);
        let f = find_dialog(&mut app);
        assert!(f.error.is_some());
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
    fn sync_with_an_archive_panel_opens_with_two_it_does_not() {
        let (_tmp, mut app, a) = zip_setup();
        listed_at(&mut app, 1, &a);
        let _ = app.update(Message::Key(Action::SyncDirs));
        assert!(matches!(app.dialog, Some(Dialog::Sync(_))));
        app.dialog = None;
        listed_at(&mut app, 0, &a.join("d"));
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
    fn diff_scroll_makes_the_block_in_view_current() {
        let (_tmp, mut app) = sync_setup();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let id = diff_dlg(&mut app).id;
        let same = "s\n".repeat(40);
        let t = shagoff_core::diff::rows(&format!("x\n{same}x\n"), &format!("y\n{same}y\n"), 100);
        let _ = app.update(Message::DiffReady(
            id,
            Arc::new(Ok(shagoff_core::diff::Outcome::Text(t))),
        ));
        assert_eq!(diff_dlg(&mut app).block, 0);
        let _ = app.update(Message::DiffScrolled(0, 0.0, 30.0 * dialogs::DIFF_ROW_H));
        assert_eq!(diff_dlg(&mut app).block, 1);
    }

    #[test]
    fn diff_sides_scroll_together() {
        let (tmp, mut app) = sync_setup();
        std::fs::write(tmp.path().join("l/a"), "x\n").unwrap();
        std::fs::write(tmp.path().join("r/a"), "y\n").unwrap();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let _ = app.update(Message::DiffScrolled(0, 120.0, 44.0)); // the left one scrolled
        let d = diff_dlg(&mut app);
        assert_eq!((d.x, d.offset), (120.0, 44.0)); // the right one is told to follow
        // ... and reports the same back: nothing more happens (no ping-pong).
        let _ = app.update(Message::DiffScrolled(1, 120.0, 44.0));
        assert_eq!(
            (diff_dlg(&mut app).x, diff_dlg(&mut app).offset),
            (120.0, 44.0)
        );
    }

    #[test]
    fn regression_closing_the_window_warns_of_unsaved_diff_copies() {
        let (tmp, mut app) = sync_setup();
        std::fs::write(tmp.path().join("l/a"), "x\n").unwrap();
        std::fs::write(tmp.path().join("r/a"), "y\n").unwrap();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        diff_dlg(&mut app).dirty = (false, true); // a block copied, not saved
        // × of the window: warns as the first Esc, the copies are not lost.
        assert!(matches!(app.on_app_exit(), Some(Message::Quit)));
        let _ = app.update(Message::Quit);
        assert!(diff_dlg(&mut app).confirm_close);
    }

    #[test]
    fn diff_edit_a_side_then_it_is_unsaved_and_undoable() {
        let (tmp, mut app) = sync_setup();
        let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
        std::fs::write(&l, "x\n").unwrap();
        std::fs::write(&r, "y\n").unwrap();
        let _ = app.update(Message::Key(Action::Down));
        app.active = 1;
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::CompareFiles));
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("x\n", "y\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        let _ = app.update(Message::DiffEdit(1));
        assert!(diff_dlg(&mut app).editing.is_some());
        // Esc leaves the editor, not the dialog.
        let _ = app.update(Message::Escape(app.window_id()));
        assert!(diff_dlg(&mut app).editing.is_none());
        let _ = app.update(Message::DiffEdit(1));
        let typed = text_editor::Action::Edit(text_editor::Edit::Insert('z'));
        let _ = app.update(Message::DiffEditAct(typed));
        let _ = app.update(Message::DiffEditDone(true));
        let d = diff_dlg(&mut app);
        assert!(d.editing.is_none() && d.result.is_none()); // comparing the edited text
        let id = d.id;
        let t = shagoff_core::diff::rows("x\n", "zy\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        let d = diff_dlg(&mut app);
        assert_eq!((d.dirty, d.undo.len()), ((false, true), 1));
        assert_eq!(std::fs::read_to_string(&r).unwrap(), "y\n"); // not saved yet
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
        let shown = diff_dlg(&mut app).id;
        let _ = app.update(Message::DiffReread); // would drop the unsaved copy: refused
        assert_eq!(diff_dlg(&mut app).id, shown);
        let _ = app.update(Message::DialogCancel);
        assert!(app.dialog.is_some()); // first Esc only warns
        let _ = app.update(Message::DiffSave);
        assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nNEW\ny\n");
        assert_eq!(std::fs::read_to_string(&l).unwrap(), "x\nNEW\ny\n");
        assert_eq!(diff_dlg(&mut app).dirty, (false, false));
        let _ = app.update(Message::DiffReread); // saved: reads both files again
        assert_ne!(diff_dlg(&mut app).id, shown);
        let _ = app.update(Message::DialogCancel);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn diff_undo_restores_text_and_dirty_until_saved() {
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
        let _ = app.update(Message::DiffUndo); // nothing copied yet
        assert_eq!(diff_dlg(&mut app).id, id);
        let _ = app.update(Message::DiffCopy(true));
        let id = diff_dlg(&mut app).id;
        let _ = app.update(Message::DiffUndo); // still copying: refused
        assert_eq!(diff_dlg(&mut app).id, id);
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        assert_eq!(diff_dlg(&mut app).dirty, (false, true));
        let _ = app.update(Message::DiffUndo);
        let d = diff_dlg(&mut app);
        assert_eq!(d.dirty, (false, false));
        assert!(d.undo.is_empty());
        assert!(d.result.is_none()); // re-comparing the texts from before the copy
        // Save drops the history: undoing after it would not match the disk.
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
        let id = diff_dlg(&mut app).id;
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        let _ = app.update(Message::DiffCopy(true));
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        assert_eq!(diff_dlg(&mut app).undo.len(), 1);
        let _ = app.update(Message::DiffSave);
        assert!(diff_dlg(&mut app).undo.is_empty());
    }

    #[test]
    fn regression_a_failed_diff_save_keeps_undo_and_the_close_warning() {
        use std::os::unix::fs::PermissionsExt;
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
        let _ = app.update(Message::DiffCopy(true));
        let id = diff_dlg(&mut app).id;
        let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
        let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
        let ro = |mode| {
            let p = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(tmp.path().join("r"), p.clone()).unwrap();
            std::fs::set_permissions(&r, p).unwrap();
        };
        ro(0o555);
        let _ = app.update(Message::DiffSave); // the right side cannot be written
        ro(0o755);
        assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nold\ny\n");
        let d = diff_dlg(&mut app);
        assert_eq!((d.dirty, d.undo.len()), ((false, true), 1)); // still undoable
        let _ = app.update(Message::DialogCancel);
        assert!(app.dialog.is_some()); // and Esc still warns first
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
        assert!(matches!(
            app.dialog,
            Some(Dialog::Input {
                op: InputOp::Mkdir,
                ..
            })
        ));
    }

    #[test]
    fn window_close_goes_the_exit_way() {
        // Alt+F4 / the title bar's ×: through `Message::Exit`, as the Exit button and menu item
        // (its temp cleanup is there).
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
        fn f12_runs_what_is_set_and_opens_the_settings_when_nothing_is() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::F12));
            assert!(matches!(app.drawer, Some(Drawer::Settings(_))));
            let _ = app.update(Message::Key(Action::F12)); // already open: stays open
            assert!(matches!(app.drawer, Some(Drawer::Settings(_))));
            let brief = (crate::toolbar::builtins().iter())
                .position(|b| b.action == Action::ViewBrief)
                .unwrap();
            let _ = app.update(Message::Setting(Setting::F12(brief + 1)));
            assert_eq!(app.config.f12.cmd, "cm_SrcShort");
            let _ = app.update(Message::CloseDrawer);
            let _ = app.update(Message::Key(Action::F12));
            assert!(app.panes[app.active].active().brief);
            // "Own program": the internal command goes, the typed one stays
            let _ = app.update(Message::Setting(Setting::F12(0)));
            assert_eq!(app.config.f12, ToolButton::default());
            let _ = app.update(Message::Setting(Setting::F12Cmd("echo".into())));
            let _ = app.update(Message::Setting(Setting::F12Params("%N".into())));
            let _ = app.update(Message::Setting(Setting::F12(0)));
            assert_eq!(
                (app.config.f12.cmd.as_str(), app.config.f12.params.as_str()),
                ("echo", "%N")
            );
        }

        #[test]
        fn view_menu_toggles_the_divider_line_and_the_frame() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::ToggleDividerLine));
            let _ = app.update(Message::Key(Action::ToggleActiveFrame));
            assert!(app.config.divider_line && app.config.active_frame);
            let _ = app.update(Message::Key(Action::ToggleDividerLine));
            assert!(!app.config.divider_line);
        }

        #[test]
        fn show_menu_switches_sizes_between_bytes_and_kb_mb_gb() {
            let mut app = app_with(Config::default(), State::default());
            // Not chosen: as the skin has them.
            assert_eq!(
                app.config.human_sizes(),
                app.config.skin == config::Skin::Modern
            );
            let was = app.config.human_sizes();
            let _ = app.update(Message::Key(Action::ToggleHumanSizes));
            assert_eq!(app.config.human_sizes, Some(!was));
            let _ = app.update(Message::Key(Action::ToggleHumanSizes));
            assert_eq!(app.config.human_sizes, Some(was));
            app.config.human_sizes = None;
            app.config.skin = config::Skin::Modern;
            assert!(app.config.human_sizes());
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
        fn connect_keep_is_off_until_ticked() {
            let mut app = app_with(Config::default(), State::default());
            let _ = app.update(Message::Key(Action::Connect));
            let keep = |app: &App| matches!(app.dialog, Some(Dialog::Connect { keep: true, .. }));
            assert!(!keep(&app));
            let _ = app.update(Message::ConnectKeep(true));
            assert!(keep(&app));
            // Connected: the dialog goes (the keyring task is never run in tests).
            let _ = app.update(Message::ConnectPassword("pw".into()));
            let _ = app.update(Message::Connected(0, "sftp://nas/".into(), Ok("/n".into())));
            assert!(app.dialog.is_none());
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
            let _ = app.update(Message::ConnectBrowse(None));
            let Some(Dialog::Connect {
                browsing: Some(stop),
                ..
            }) = &app.dialog
            else {
                panic!("not browsing")
            };
            let _ = app.update(Message::Browsed(stop.clone(), Ok(found.clone())));
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
        fn browse_stops_by_its_button_or_by_closing_the_dialog() {
            let mut app = app_with(Config::default(), State::default());
            let stop = |app: &App| match &app.dialog {
                Some(Dialog::Connect { browsing, .. }) => browsing.clone(),
                _ => panic!("no dialog"),
            };
            let _ = app.update(Message::Key(Action::Connect));
            let _ = app.update(Message::ConnectBrowse(None));
            let first = stop(&app).expect("browsing");
            let _ = app.update(Message::ConnectBrowse(None)); // pressed again: stop
            assert!(first.load(Ordering::Relaxed) && stop(&app).is_none());
            // Its "cancelled" lands later: no error shown.
            let _ = app.update(Message::Browsed(first, Err(MountError::Cancelled)));
            assert!(matches!(
                &app.dialog,
                Some(Dialog::Connect { note: None, .. })
            ));
            let _ = app.update(Message::ConnectBrowse(None));
            let second = stop(&app).expect("browsing");
            let _ = app.update(Message::DialogCancel);
            assert!(second.load(Ordering::Relaxed));
        }

        #[test]
        fn browse_goes_inside_and_drops_a_stale_result() {
            let mut app = app_with(Config::default(), State::default());
            let place = |app: &App| match &app.dialog {
                Some(Dialog::Connect {
                    at,
                    found,
                    browsing,
                    ..
                }) => (at.clone(), found.clone(), browsing.clone()),
                _ => panic!("no dialog"),
            };
            let _ = app.update(Message::Key(Action::Connect));
            let _ = app.update(Message::ConnectBrowse(None));
            let net = vec![("Windows Network".to_string(), "smb:///".to_string())];
            let stop = place(&app).2.expect("browsing");
            let _ = app.update(Message::Browsed(stop, Ok(net.clone())));
            assert_eq!(place(&app).1, net);
            // Deeper: the old list goes at once; deeper again before it answers.
            let _ = app.update(Message::ConnectBrowse(Some("smb:///".into())));
            let first = place(&app).2.expect("browsing");
            let _ = app.update(Message::ConnectBrowse(Some("smb://WG/".into())));
            let (at, found, second) = place(&app);
            assert_eq!((at.as_str(), found.len()), ("smb://WG/", 0));
            assert!(first.load(Ordering::Relaxed));
            // The first one finishing late anyway is not shown, the second still stoppable.
            let wg = vec![("WG".to_string(), "smb://WG/".to_string())];
            let _ = app.update(Message::Browsed(first, Ok(wg)));
            assert!(place(&app).1.is_empty() && place(&app).2.is_some());
            let nas = vec![("NAS".to_string(), "smb://nas/".to_string())];
            let _ = app.update(Message::Browsed(second.expect("browsing"), Ok(nas.clone())));
            let (_, found, browsing) = place(&app);
            assert_eq!((found, browsing.is_none()), (nas, true));
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
            assert!(typed("y", Code::KeyY, Modifiers::CTRL).is_none());
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
        fn names_not_in_utf8_go_in_as_a_glob() {
            use std::os::unix::ffi::OsStrExt;
            let tmp = tempfile::tempdir().unwrap();
            let bad = std::ffi::OsStr::from_bytes(b"caf\xe9");
            std::fs::write(tmp.path().join(bad), "").unwrap();
            let mut app = app_with(Config::default(), State::default());
            listed_at(&mut app, 0, tmp.path());
            let panel = &mut app.panes[0].active_mut().panel;
            let i = (panel.entries().iter())
                .position(|e| e.os_name == bad)
                .unwrap();
            panel.set_cursor(i);
            let _ = app.update(Message::Key(Action::CmdName));
            assert_eq!(app.cmdline, "caf?");
            // The same pattern fits "cafe" too (by characters): not inserted.
            app.cmdline.clear();
            std::fs::write(tmp.path().join("café"), "").unwrap();
            listed_at(&mut app, 0, tmp.path());
            let panel = &mut app.panes[0].active_mut().panel;
            let i = (panel.entries().iter())
                .position(|e| e.os_name == bad)
                .unwrap();
            panel.set_cursor(i);
            let _ = app.update(Message::Key(Action::CmdName));
            assert_eq!(app.cmdline, "");
            assert_eq!(app.msg(), Some(fl!("cmd-name-ambiguous").as_str()));
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
            // Ctrl+X in the line: the next command, at the newest it stays.
            let _ = app.update(Message::FieldKey(Action::ClipCut));
            assert_eq!(app.cmdline, "make");
            let _ = app.update(Message::FieldKey(Action::ClipCut));
            assert_eq!(app.cmdline, "make");
            // A typed line stays; the panel's Ctrl+X / menu «Cut» never step the history.
            app.cmdline = "mak".into();
            let _ = app.update(Message::FieldKey(Action::ClipCut));
            assert_eq!(app.cmdline, "mak");
            app.cmdline = "ls".into();
            let _ = app.update(Message::Key(Action::ClipCut));
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

        #[test]
        fn regression_ctrl_up_on_dotdot_in_results_opens_the_searched_dir() {
            let (tmp, mut app) = setup();
            let f = tmp.path().join("f");
            let t = app.panes[0].active_mut();
            t.results = Some((tmp.path().to_path_buf(), Arc::new(vec![f.clone()])));
            let rows =
                read_listing(tmp.path(), false, Some(&vec![f]), &AtomicBool::new(false)).unwrap();
            t.panel.set_listing(tmp.path().to_path_buf(), rows, None);
            key(&mut app, Action::Home); // ".."
            key(&mut app, Action::TabOpen);
            // As Backspace there: the dir that was searched, not its parent.
            assert_eq!(app.panes[0].active().target(), tmp.path());
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
        fn a_tab_locked_with_changes_moves_and_goes_back_when_left() {
            let (tmp, mut app) = setup();
            key(&mut app, Action::TabRoam);
            let t = app.panes[0].active();
            assert_eq!((t.home.as_deref(), t.locked), (Some(tmp.path()), false));
            key(&mut app, Action::CloseTab); // not closed, as locked
            assert_eq!(app.panes[0].items().len(), 1);
            key(&mut app, Action::Enter); // into `a`: in this very tab
            assert_eq!(app.panes[0].items().len(), 1);
            listed_at(&mut app, 0, &tmp.path().join("a"));
            assert!(app.panes[0].active().title().starts_with('!'));
            // Another tab shown: this one is read in its dir again.
            key(&mut app, Action::NewTab);
            let first = &app.panes[0].items()[0];
            assert_eq!(first.target(), tmp.path());
            // Saved: it opens there, still with dir changes allowed.
            app.panes[0].select(0);
            listed_at(&mut app, 0, &tmp.path().join("a"));
            app.save_state();
            let saved = &app.saved.panes[0];
            assert_eq!((saved.tabs[0].as_path(), saved.roam[0]), (tmp.path(), true));
            // Plain lock and this one exclude each other.
            key(&mut app, Action::TabLock);
            assert!(app.panes[0].active().locked && app.panes[0].active().home.is_none());
        }

        #[test]
        fn ctrl_up_keeps_the_sort_of_its_tab() {
            let (_tmp, mut app) = setup();
            key(&mut app, Action::Sort(shagoff_core::sort::SortKey::Size));
            let sort = app.panes[0].active().panel.sort();
            key(&mut app, Action::TabOpen);
            assert_eq!(app.panes[0].active().panel.sort(), sort);
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
