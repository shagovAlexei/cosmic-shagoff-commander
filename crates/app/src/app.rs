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
use shagoff_core::panel::{self, Like, PARENT, Panel, Show};
use shagoff_core::repack::Change;
use shagoff_core::session::{self, PaneState};
use shagoff_core::split;
use shagoff_core::tabs::Tabs;
use shagoff_core::templates;
use shagoff_core::terminals::{self, Terminal};
use shagoff_core::trashcan;
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
    /// Commands → «Synchronous dir change»: the other panel goes along (`panel::sync_step`).
    pub sync_dirs: bool,
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
    /// The path line being edited: its pane and text (`PathEditStart`).
    pub(crate) path_edit: Option<(usize, String)>,
    pub(crate) path_id: widget::Id,
    pub commands: Vec<String>,
    /// Num+ / Num− masks, last first: the dialog opens with the last, ↑ / ↓ go through them.
    pub masks: Vec<String>,
    /// "Open with…" → "Other program…" commands, last first; ↑ / ↓ as for masks.
    pub other_cmds: Vec<String>,
    /// Alt+Shift+F9 under way: another waits for it.
    testing: bool,
}

/// Mark → «Save selection to file…»: the file offered, in the panel's dir.
const MARKS_FILE: &str = "selection.txt";

/// Mark → «Save details to file…»: the file offered.
const DETAILS_FILE: &str = "details.txt";

fn marks_file(panel: &Panel) -> String {
    offered_file(panel, MARKS_FILE)
}

/// Where a file of ours goes: the panel's dir; inside an archive, next to it (that is no place
/// to write).
fn writable_dir(panel: &Panel) -> PathBuf {
    match archive::split_path(panel.cwd()) {
        Some((arc, _)) => arc.parent().map(Path::to_path_buf).unwrap_or_default(),
        None => panel.cwd().to_path_buf(),
    }
}

fn offered_file(panel: &Panel, name: &str) -> String {
    writable_dir(panel).join(name).display().to_string()
}

/// Write `text` to a new file: one that is there is not written over (TC asks; here: refused).
// ponytail: on the UI thread, a few KB; a dead mount hangs it.
fn write_new(file: &Path, text: &str) -> std::io::Result<()> {
    std::io::Write::write_all(&mut std::fs::File::create_new(file)?, text.as_bytes())
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
    /// Files → «Split file»: the part size typed.
    SplitSize(String),
    /// Split / Combine done: what to say (`Ok`) or the error.
    Pieces(Result<String, String>),
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
    /// Items put back from the trash: the ones that could not be, with why.
    Restored(Vec<String>),
    /// «Empty trash»: everything in every trash, to ask about (side), or why it can't be read.
    TrashListed(usize, Result<Vec<PathBuf>, String>),
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
    /// Mark → «Selection from the clipboard»: its text, for the active panel.
    MarksText(usize, Option<String>),
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
    /// The path line as a field (TC `cm_EditPath`): opened on this pane, typed, Enter.
    PathEditStart(usize),
    PathEdit(String),
    PathEditGo,
    /// The field lost the focus (TC leaves the edit then).
    PathEditCancel,
    /// Where Enter goes, found off the UI thread: the pane, the path, the file to put the cursor on.
    PathEditFound(usize, PathBuf, Option<String>),
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
        if self.job.is_none() && self.config.watch_dirs {
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
            sync_dirs: self.sync_dirs,
            only_marked: self.panes[self.active].active().panel.only_marked(),
            ignore: c.ignore_on,
            watch: c.watch_dirs,
            dirs_by_name: c.dirs_by_name,
            type_colors: c.type_colors,
            start: (c.start_menu.iter())
                .map(|b| (b.cmd != crate::toolbar::SEP).then(|| crate::toolbar::tip(b)))
                .collect(),
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
        config.set_core_switches();
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
            scheme: config.scheme(),
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
            sync_dirs: false,
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
            path_edit: None,
            path_id: widget::Id::unique(),
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
}

mod act;
mod cmd;
mod dialog_flow;
mod handle;
mod job_flow;
mod nav;
mod settings;
mod tools;
mod ui;
mod util;
mod viewer;
use util::*;

#[cfg(test)]
mod tests;
