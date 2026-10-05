//! Modal dialogs: one enum, one view per variant.

use crate::app::{Message, OpKind, Running};
use crate::fl;
use cosmic::iced::Length;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::widget::{column, row};
use cosmic::{Element, widget};
use shagoff_core::archive::Format;
use shagoff_core::diff;
use shagoff_core::format::{self, TimeZone};
use shagoff_core::multirename::{self, Case, Counter, Problem, Row, Rule};
use shagoff_core::ops::{self, ErrorChoice, FileInfo, Resolution};
use shagoff_core::sync;
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOp {
    Copy,
    Move,
    Mkdir,
    Rename,
    /// Own caption of the active tab.
    TabName,
}

pub enum Dialog {
    /// Num+ / Num−.
    Mask {
        side: usize,
        select: bool,
        input: String,
    },
    /// F5 / F6 target, F7 name, Shift+F6 new name.
    Input {
        op: InputOp,
        side: usize,
        sources: Vec<PathBuf>,
        input: String,
    },
    /// F8 / Shift+F8.
    ConfirmDelete {
        side: usize,
        permanent: bool,
        paths: Vec<PathBuf>,
    },
    /// Asked by a running job; the worker blocks until `reply` gets an answer (or is dropped → Cancel).
    Conflict {
        src: FileInfo,
        dst: FileInfo,
        reply: mpsc::Sender<Resolution>,
        /// "Rename": the name to write this file under; starts as the first free `name (N).ext`.
        name: String,
    },
    Error {
        path: PathBuf,
        error: String,
        reply: mpsc::Sender<ErrorChoice>,
    },
    /// Alt+F1/F2, Alt+↓, Ctrl+D: a snapshot taken when the list opened.
    List {
        kind: ListKind,
        side: usize,
        cursor: usize,
        items: Vec<ListItem>,
    },
    Props(Box<Props>),
    /// Ctrl+D → "Configure…".
    Hotlist(Box<crate::hotlist::HotEdit>),
    /// Ctrl+M: files snapshot at open (panel order) and the form. Boxed: the form is large.
    MultiRename(Box<MultiRename>),
    /// Alt+F5. Boxed: the form is large.
    Pack(Box<Pack>),
    /// Alt+F9: the archives among the targets at open.
    Unpack {
        side: usize,
        archives: Vec<PathBuf>,
        path: String,
        own_dir: bool,
    },
    /// Alt+F7. Boxed: the form and results are large.
    Find(Box<Find>),
    /// Ctrl+Shift+S.
    Sync(Box<SyncDlg>),
    /// Ctrl+Shift+D.
    Diff(Box<DiffDlg>),
    /// A file from an archive was saved in the editor (F4): put it back?
    UpdateArchive {
        file: PathBuf,
        archive: PathBuf,
        entry: PathBuf,
    },
    /// Ctrl+F: network location and password.
    Connect {
        side: usize,
        url: String,
        password: String,
        /// Saved addresses when the dialog opened.
        saved: Vec<String>,
        /// "Browse network": (name, address).
        found: Vec<(String, String)>,
        browsing: bool,
        /// Shown in the dialog (as in TC, not in the status line): "Connecting…", or the
        /// last error (`true`) / browse result. The dialog stays open until connected.
        note: Option<(bool, String)>,
    },
}

pub struct DiffDlg {
    pub left: PathBuf,
    pub right: PathBuf,
    /// Results of other compares are dropped.
    pub id: u64,
    /// `None` while comparing. `Arc`: `Message` must be `Clone` and the rows are large.
    pub result: Option<Arc<Result<diff::Outcome, String>>>,
    /// Current block of differences (index into `blocks()`).
    pub block: usize,
    pub scroll: widget::Id,
    /// Vertical scroll offset: only the rows in view are built.
    pub offset: f32,
    /// Width of each side, from its longest line: `Fill` inside a two-way scrollable lays out at 0.
    pub widths: (f32, f32),
    pub opts: diff::Opts,
    /// Changed by copying blocks, not saved yet (left, right).
    pub dirty: (bool, bool),
    /// Esc / Cancel was pressed once with unsaved changes: the next one closes without saving.
    pub confirm_close: bool,
    /// Size and mtime of both files when read: saving refuses if they changed since.
    pub stamps: Option<[diff::Stamp; 2]>,
    /// A block copy is computing: (to the right, the texts before it).
    pub copying: Option<(bool, Arc<(String, String)>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffOpt {
    Space,
    Case,
}

/// Diff rows have one fixed height, so a block's scroll offset is `row * DIFF_ROW_H`.
pub const DIFF_ROW_H: f32 = 22.0;
pub const DIFF_LIST_H: f32 = 260.0;
/// Monospace advance at the default text size, plus the line-number column.
const MONO_W: f32 = 8.5;
const NUM_W: f32 = 56.0;

impl DiffDlg {
    pub fn text(&self) -> Option<&diff::Text> {
        match self.result.as_deref() {
            Some(Ok(diff::Outcome::Text(t))) => Some(t),
            _ => None,
        }
    }

    /// Blocks that start within the kept rows (the only ones we can scroll to).
    pub fn blocks(&self) -> &[usize] {
        self.text().map_or(&[], |t| {
            let n = t.blocks.partition_point(|&b| b < t.rows.len());
            &t.blocks[..n]
        })
    }

    /// Side widths from the longest line on each side (at least a readable minimum).
    pub fn measure(t: &diff::Text) -> (f32, f32) {
        let longest = |f: fn(&diff::Row) -> &Option<(usize, String)>| {
            t.rows
                .iter()
                .filter_map(|r| f(r).as_ref())
                .map(|(_, s)| expand(s).chars().count())
                .max()
                .unwrap_or(0)
        };
        let w = |chars: usize| (NUM_W + chars as f32 * MONO_W).max(420.0);
        (w(longest(|r| &r.left)), w(longest(|r| &r.right)))
    }
}

fn expand(s: &str) -> String {
    s.replace('\t', "    ")
}

pub struct SyncDlg {
    pub side: usize,
    pub left: PathBuf,
    pub right: PathBuf,
    pub recursive: bool,
    pub content: bool,
    pub ignore_date: bool,
    pub hidden: bool,
    /// TC's show buttons: kinds of rows hidden from the list (✕ rows are always listed).
    pub hide: HashSet<sync::Kind>,
    /// The right becomes a copy of the left (right-only goes to the trash).
    pub mirror: bool,
    /// Mask of file names to compare, as typed ("" or "*": all).
    pub mask: String,
    /// "Synchronize" was pressed once with deletions pending: the next press runs.
    pub confirm: bool,
    pub rows: Vec<sync::Row>,
    /// Results of other compares are dropped.
    pub id: u64,
    /// Set while a compare runs.
    pub running: Option<Arc<AtomicBool>>,
}

impl SyncDlg {
    pub fn options(&self) -> sync::Options {
        sync::Options {
            recursive: self.recursive,
            content: self.content,
            ignore_date: self.ignore_date,
            hidden: self.hidden,
            mirror: self.mirror,
            mask: match self.mask.trim() {
                "" | "*" | "*.*" => None,
                m => Some(shagoff_core::mask::Mask::parse(m)),
            },
        }
    }
}

impl Drop for SyncDlg {
    fn drop(&mut self) {
        if let Some(s) = &self.running {
            s.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncOpt {
    Recursive,
    Content,
    IgnoreDate,
    Mirror,
}

/// Deleted (left only), inserted (right only), changed: tinted, readable in light and dark themes.
fn diff_style(kind: diff::Kind) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |t| {
        let c = t.cosmic();
        let tint = match kind {
            diff::Kind::Same => return Default::default(),
            diff::Kind::Deleted => c.destructive_color(),
            diff::Kind::Inserted => c.success_color(),
            diff::Kind::Changed => c.accent_color(),
        };
        let mut bg = cosmic::iced::Color::from(tint);
        bg.a = 0.25;
        cosmic::iced::widget::container::Style {
            background: Some(bg.into()),
            ..Default::default()
        }
    })
}

/// Result lists grow with their rows up to a cap, so an empty list leaves no blank area and a
/// full one still fits a short window.
fn list_height(rows: usize) -> Length {
    Length::Fixed((rows as f32 * 36.0).min(240.0))
}

/// Rows kept and shown; the search still counts everything.
pub const FIND_SHOWN: usize = 1000;

pub struct Find {
    pub side: usize,
    pub mask: String,
    pub dir: String,
    pub text: String,
    pub case_sensitive: bool,
    /// The text is a regular expression.
    pub regex: bool,
    /// The mask field is a regular expression for the name.
    pub name_regex: bool,
    /// Also names inside zip / tar / 7z.
    pub archives: bool,
    /// Filters as typed: size bounds in KB, "not older than" in days.
    pub min_size: String,
    pub max_size: String,
    pub days: String,
    /// Why the search did not start (a bad filter or regex).
    pub error: Option<String>,
    /// The dir the last search ran in ("To panel" lists it there).
    pub root: Option<PathBuf>,
    /// Every match; the list shows the first `FIND_SHOWN`, "To panel" takes them all.
    pub results: Vec<PathBuf>,
    pub total: usize,
    /// Dir being searched (progress line).
    pub current: String,
    pub cursor: usize,
    /// Events of other searches are dropped.
    pub id: u64,
    /// Set while a search runs.
    pub stop: Option<Arc<AtomicBool>>,
    /// ↑/↓ moved into the results: Enter (a field's submit) goes to the file, not a new search.
    pub in_list: bool,
}

/// Closing the dialog in any way stops its search.
/// Alt+Enter on the selection.
pub struct Props {
    pub side: usize,
    pub paths: Vec<PathBuf>,
    /// (label, value) lines above the bits: name, type, date, owner…
    pub facts: Vec<(String, String)>,
    /// Bits set on every entry, and as toggled since.
    pub mode: u32,
    /// Bits shown as `?` until touched: they differ between the entries, or (recursive)
    /// every bit, as TC's grey "leave as is" boxes — the contents may differ from the dir.
    pub mixed: u32,
    /// `mixed` of the selection itself, for when "recursive" goes off again.
    pub own_mixed: u32,
    /// Bits the user toggled: exactly these are set or cleared on every entry.
    pub touched: u32,
    /// A dir is selected: offer to change what is inside too.
    pub has_dir: bool,
    pub recursive: bool,
    /// Counted in the background; `None` while counting.
    pub usage: Option<shagoff_core::props::Usage>,
    pub id: u64,
    pub stop: Arc<AtomicBool>,
}

impl Drop for Props {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Find {
    fn drop(&mut self) {
        if let Some(s) = &self.stop {
            s.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindField {
    Mask,
    Dir,
    Text,
    MinSize,
    MaxSize,
    Days,
}

pub struct Pack {
    pub side: usize,
    pub sources: Vec<PathBuf>,
    /// The panel dir: names inside the archive are relative to it.
    pub base: PathBuf,
    pub path: String,
    pub format: Format,
    pub move_after: bool,
    pub separate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    MoveAfter,
    Separate,
    OwnDir,
}

pub struct MultiRename {
    pub side: usize,
    pub dir: PathBuf,
    pub files: Vec<(String, SystemTime)>,
    /// Names in `dir` that are not being renamed.
    pub taken: HashSet<String>,
    /// The entry under the cursor at open, if it is being renamed: the cursor follows it.
    pub current: Option<String>,
    /// Masks, find/replace and case; the counter comes from the text fields below.
    pub rule: Rule,
    pub start: String,
    pub step: String,
    pub digits: String,
}

impl MultiRename {
    /// Counter fields that don't parse fall back to the defaults.
    pub fn rule(&self) -> Rule {
        let d = Counter::default();
        Rule {
            counter: Counter {
                start: self.start.trim().parse().unwrap_or(d.start),
                step: self.step.trim().parse().unwrap_or(d.step),
                digits: self.digits.trim().parse().unwrap_or(d.digits),
            },
            ..self.rule.clone()
        }
    }

    pub fn rows(&self, tz: &TimeZone) -> Vec<Row> {
        multirename::preview(&self.rule(), &self.files, &self.taken, tz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MrField {
    Name,
    Ext,
    Find,
    Replace,
    Start,
    Step,
    Digits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Drives,
    History,
    Hotlist,
    /// Command line history (Alt+F8).
    Commands,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub label: String,
    /// Empty for the hotlist's own rows.
    pub path: PathBuf,
    pub kind: Item,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Item {
    #[default]
    Dir,
    /// An unmounted volume: `path` is its device, Enter mounts it.
    Mount,
    /// A line between groups; Enter does nothing.
    Sep,
    /// Hotlist: add the panel's dir.
    Add,
    /// Hotlist: open the settings.
    Configure,
}

impl Dialog {
    /// The text field, for dialogs that have one.
    pub fn input_mut(&mut self) -> Option<&mut String> {
        match self {
            Dialog::Mask { input, .. } | Dialog::Input { input, .. } => Some(input),
            Dialog::Pack(p) => Some(&mut p.path),
            Dialog::Unpack { path, .. } => Some(path),
            Dialog::Connect { url, .. } => Some(url),
            Dialog::Conflict { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// One row of a TC-style menu list: name, then a path cut with "…"; the selection looks
/// like the panel cursor.
pub fn menu_row<'a>(
    name: String,
    path: String,
    selected: bool,
    on: Message,
) -> Element<'a, Message> {
    use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit};
    let cut = |t: widget::Text<'a, cosmic::Theme>, portion| {
        widget::container(
            t.wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
        )
        .width(Length::FillPortion(portion))
        .clip(true)
        // Empty style: the default one sets its own text colour over the selection's.
        .class(cosmic::theme::Container::custom(|_| Default::default()))
    };
    let line = row![
        // Plain `text`: body / caption carry their own colour, not the cursor's.
        cut(widget::text(name).size(14), 2),
        cut(widget::text(path).size(12), 3)
    ]
    .spacing(12);
    widget::button::custom(
        widget::container(line)
            .padding([4, 8])
            .width(Length::Fill)
            .height(Length::Fixed(28.0))
            .align_y(cosmic::iced::Alignment::Center)
            .class(crate::view::cursor_style(
                selected,
                true,
                false,
                crate::config::Skin::Modern,
            )),
    )
    .padding(0)
    .width(Length::Fill)
    .class(cosmic::theme::Button::MenuItem)
    .on_press(on)
    .into()
}

fn problem(p: Problem) -> String {
    match p {
        Problem::BadName => fl!("mr-bad-name"),
        Problem::Duplicate => fl!("mr-duplicate"),
        Problem::Exists => fl!("mr-exists"),
    }
}

/// "a.txt" for one path, "3 files" for several.
fn what(paths: &[PathBuf]) -> String {
    match paths {
        [one] => one
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => fl!("n-files", n = paths.len()),
    }
}

pub fn view<'a>(d: &'a Dialog, input_id: &widget::Id, tz: &TimeZone) -> Element<'a, Message> {
    let cancel = widget::button::standard(fl!("cancel")).on_press(Message::DialogCancel);
    let ok = widget::button::suggested(fl!("ok")).on_press(Message::DialogSubmit);
    let field = |value: &'a str| {
        widget::text_input("", value)
            .id(input_id.clone())
            .on_input(Message::DialogInput)
            .on_submit(|_| Message::DialogSubmit)
    };
    match d {
        Dialog::Mask { select, input, .. } => widget::dialog()
            .title(if *select {
                fl!("select-group")
            } else {
                fl!("unselect-group")
            })
            .control(field(input))
            .primary_action(ok)
            .secondary_action(cancel)
            .into(),
        Dialog::Input {
            op, sources, input, ..
        } => {
            let title = match op {
                InputOp::Copy => fl!("copy-to", what = what(sources)),
                InputOp::Move => fl!("move-to", what = what(sources)),
                InputOp::Mkdir => fl!("mkdir"),
                InputOp::Rename => fl!("rename"),
                InputOp::TabName => fl!("tab-rename"),
            };
            widget::dialog()
                .title(title)
                .control(field(input))
                .primary_action(ok)
                .secondary_action(cancel)
                .into()
        }
        Dialog::UpdateArchive { archive, entry, .. } => widget::dialog()
            .title(fl!("archive-update-title"))
            .body(fl!(
                "archive-update",
                entry = entry.display().to_string(),
                archive = archive.display().to_string()
            ))
            .primary_action(
                widget::button::suggested(fl!("archive-update-ok")).on_press(Message::DialogSubmit),
            )
            .secondary_action(cancel)
            .into(),
        Dialog::ConfirmDelete {
            permanent, paths, ..
        } => {
            let (body, button) = if *permanent {
                (
                    fl!("delete-permanent", what = what(paths)),
                    widget::button::destructive(fl!("delete")),
                )
            } else {
                (
                    fl!("delete-trash", what = what(paths)),
                    widget::button::suggested(fl!("delete")),
                )
            };
            widget::dialog()
                .body(body)
                .primary_action(button.on_press(Message::DialogSubmit))
                .secondary_action(cancel)
                .into()
        }
        Dialog::Conflict { src, dst, name, .. } => {
            let line = |key: &str, f: &FileInfo| {
                let (size, date) = (format::size(f.size), format::date(f.mtime, tz));
                match key {
                    "new" => fl!("new-file", size = size, date = date),
                    _ => fl!("existing-file", size = size, date = date),
                }
            };
            let answer = |label: String, r: Resolution| {
                widget::button::standard(label).on_press(Message::Resolve(r))
            };
            widget::dialog()
                .title(fl!("file-exists"))
                .body(format!(
                    "{}\n{}\n{}",
                    dst.path.display(),
                    line("new", src),
                    line("existing", dst)
                ))
                .control(
                    column![
                        row![
                            answer(fl!("skip"), Resolution::Skip),
                            answer(fl!("skip-all"), Resolution::SkipAll),
                        ]
                        .spacing(8),
                        row![
                            answer(fl!("replace-all"), Resolution::ReplaceAll),
                            answer(fl!("replace-older"), Resolution::ReplaceOlder),
                        ]
                        .spacing(8),
                        row![
                            widget::text_input("", name.as_str())
                                .on_input(Message::DialogInput)
                                .width(Length::Fill),
                            widget::button::standard(fl!("rename-to")).on_press_maybe(
                                ops::valid_name(name)
                                    .then(|| Message::Resolve(Resolution::Rename(name.clone())))
                            ),
                            answer(fl!("rename-all"), Resolution::RenameAll),
                        ]
                        .spacing(8)
                        .align_y(cosmic::iced::Alignment::Center),
                    ]
                    .spacing(8),
                )
                .primary_action(
                    widget::button::suggested(fl!("replace"))
                        .on_press(Message::Resolve(Resolution::Replace)),
                )
                .secondary_action(
                    widget::button::standard(fl!("cancel"))
                        .on_press(Message::Resolve(Resolution::Cancel)),
                )
                .into()
        }
        Dialog::Hotlist(h) => crate::hotlist::view(h, cancel.into()),
        Dialog::Props(p) => {
            const LABEL: f32 = 150.0;
            const CELL: f32 = 70.0;
            let line = |k: String, v: String| {
                row![
                    widget::text::body(k).width(Length::Fixed(LABEL)),
                    widget::text::body(v).wrapping(Wrapping::WordOrGlyph),
                ]
                .spacing(8)
            };
            let mut body = column![].spacing(4);
            for (k, v) in &p.facts {
                body = body.push(line(k.clone(), v.clone()));
            }
            let size = match p.usage {
                None => fl!("props-counting"),
                Some(u) => fl!(
                    "props-usage",
                    size = format::size(u.bytes),
                    files = u.files,
                    dirs = u.dirs
                ),
            };
            body = body.push(line(fl!("props-size"), size));
            let bit = |b: u32| {
                let unknown = p.mixed & !p.touched & b != 0;
                let cell: Element<'a, Message> = if unknown {
                    // "Leave as is": the first click turns it on, then it is a plain box.
                    widget::button::text("?")
                        .padding([0, 4])
                        .height(Length::Fixed(20.0))
                        .on_press(Message::PropsBit(b))
                        .into()
                } else {
                    widget::checkbox(p.mode & b != 0)
                        .on_toggle(move |_| Message::PropsBit(b))
                        .into()
                };
                // One height for both kinds, so the rows don't jump when `?` turns into a box.
                widget::container(cell)
                    .width(Length::Fixed(CELL))
                    .height(Length::Fixed(24.0))
                    .align_y(cosmic::iced::Alignment::Center)
            };
            let head = |s: String| widget::text::caption(s).width(Length::Fixed(CELL));
            let mut bits = column![
                row![
                    widget::Space::new().width(Length::Fixed(LABEL)),
                    head(fl!("props-read")),
                    head(fl!("props-write")),
                    head(fl!("props-exec")),
                ]
                .spacing(8)
            ]
            .spacing(4);
            for (who, shift) in [
                (fl!("props-owner"), 6),
                (fl!("props-group"), 3),
                (fl!("props-others"), 0),
            ] {
                bits = bits.push(
                    row![
                        widget::text::body(who).width(Length::Fixed(LABEL)),
                        bit(4 << shift),
                        bit(2 << shift),
                        bit(1 << shift),
                    ]
                    .spacing(8),
                );
            }
            let unknown = p.mixed & !p.touched;
            let shown: String = format::perms(p.mode)
                .chars()
                .enumerate()
                .map(|(i, c)| if unknown & (0o400 >> i) != 0 { '?' } else { c })
                .collect();
            bits = bits.push(widget::text::caption(match unknown {
                0 => format!("{shown} ({:o})", p.mode & 0o7777),
                _ => shown,
            }));
            let mut control = column![body, bits].spacing(16);
            if p.has_dir {
                control = control.push(
                    widget::checkbox(p.recursive)
                        .label(fl!("props-recursive"))
                        .on_toggle(|_| Message::PropsRecursive),
                );
            }
            widget::dialog()
                .title(fl!("props-title"))
                .control(widget::scrollable(control).height(Length::Shrink))
                .primary_action(
                    widget::button::suggested(fl!("props-apply")).on_press(Message::DialogSubmit),
                )
                .secondary_action(
                    widget::button::standard(fl!("cancel")).on_press(Message::DialogCancel),
                )
                .into()
        }
        Dialog::List {
            kind,
            cursor,
            items,
            ..
        } => {
            // Like a TC popup menu: name, then the path dimmed and cut, one line per row.
            let mut list = column![].spacing(1);
            for (i, item) in items.iter().enumerate() {
                if item.kind == Item::Sep {
                    list = list.push(
                        widget::container(widget::divider::horizontal::default()).padding([4, 8]),
                    );
                    continue;
                }
                let path = match item.kind {
                    Item::Mount => format!("{}   ({})", item.path.display(), fl!("not-mounted")),
                    _ => item.path.display().to_string(),
                };
                list = list.push(menu_row(
                    item.label.clone(),
                    path,
                    i == *cursor,
                    Message::ListPick(i),
                ));
            }
            let list = widget::scrollable(list).height(Length::Shrink);
            let title = match kind {
                ListKind::Drives => fl!("drives"),
                ListKind::History => fl!("history"),
                ListKind::Hotlist => fl!("hotlist"),
                ListKind::Commands => fl!("cmd-history"),
            };
            widget::dialog()
                .title(title)
                .control(list)
                .secondary_action(cancel)
                .into()
        }
        Dialog::MultiRename(m) => {
            let edit = |label: String, value: &'a str, f: MrField| {
                column![
                    widget::text::caption(label),
                    widget::text_input("", value)
                        .on_input(move |s| Message::MrInput(f, s))
                        .on_submit(|_| Message::DialogSubmit),
                ]
                .spacing(2)
            };
            let name = column![
                widget::text::caption(fl!("mr-name")),
                widget::text_input("", &m.rule.name)
                    .id(input_id.clone())
                    .on_input(|s| Message::MrInput(MrField::Name, s))
                    .on_submit(|_| Message::DialogSubmit),
            ]
            .spacing(2);
            let case = |label: String, c: Case| {
                let b = if m.rule.case == c {
                    widget::button::suggested(label)
                } else {
                    widget::button::standard(label)
                };
                b.on_press(Message::MrCase(c))
            };
            let mut table = column![
                row![
                    widget::text::heading(fl!("mr-old")).width(Length::FillPortion(1)),
                    widget::text::heading(fl!("mr-new")).width(Length::FillPortion(1)),
                ]
                .spacing(16)
            ]
            .spacing(2);
            let rows = m.rows(tz);
            for r in &rows {
                let new = match r.problem {
                    None => r.new.clone(),
                    Some(p) => format!("⚠ {}  ({})", r.new, problem(p)),
                };
                // Long names have no spaces to break at: wrap by glyph instead of overlapping.
                table = table.push(
                    row![
                        widget::text(r.old.clone())
                            .wrapping(Wrapping::WordOrGlyph)
                            .width(Length::FillPortion(1)),
                        widget::text(new)
                            .wrapping(Wrapping::WordOrGlyph)
                            .width(Length::FillPortion(1)),
                    ]
                    .spacing(16),
                );
            }
            let ok = rows.iter().all(|r| r.problem.is_none());
            widget::dialog()
                .title(fl!("multi-rename"))
                // Wider than the default 570 px: two columns of file names.
                .width(Length::Fill)
                .max_width(1100.0)
                .control(
                    column![
                        row![name, edit(fl!("mr-ext"), &m.rule.ext, MrField::Ext)].spacing(8),
                        row![
                            edit(fl!("mr-find"), &m.rule.find, MrField::Find),
                            edit(fl!("mr-replace"), &m.rule.replace, MrField::Replace),
                        ]
                        .spacing(8),
                        row![
                            edit(fl!("mr-start"), &m.start, MrField::Start),
                            edit(fl!("mr-step"), &m.step, MrField::Step),
                            edit(fl!("mr-digits"), &m.digits, MrField::Digits),
                        ]
                        .spacing(8),
                        row![
                            case(fl!("mr-case-keep"), Case::Keep),
                            case(fl!("mr-case-upper"), Case::Upper),
                            case(fl!("mr-case-lower"), Case::Lower),
                            case(fl!("mr-case-title"), Case::Title),
                        ]
                        .spacing(8),
                        widget::scrollable(table).height(Length::Fixed(300.0)),
                    ]
                    .spacing(12),
                )
                .primary_action(
                    widget::button::suggested(fl!("rename"))
                        .on_press_maybe(ok.then_some(Message::DialogSubmit)),
                )
                .secondary_action(cancel)
                .into()
        }
        Dialog::Pack(p) => {
            let format = |f: Format| {
                let b = if p.format == f {
                    widget::button::suggested(f.ext())
                } else {
                    widget::button::standard(f.ext())
                };
                b.on_press(Message::PackFormat(f))
            };
            let formats = Format::PACK
                .into_iter()
                .fold(row![].spacing(8), |r, f| r.push(format(f)));
            widget::dialog()
                .title(fl!("pack-to", what = what(&p.sources)))
                .control(
                    column![
                        field(&p.path),
                        formats,
                        widget::checkbox(p.move_after)
                            .label(fl!("pack-move"))
                            .on_toggle(|_| Message::Toggle(Toggle::MoveAfter)),
                        widget::checkbox(p.separate)
                            .label(fl!("pack-separate"))
                            .on_toggle(|_| Message::Toggle(Toggle::Separate)),
                    ]
                    .spacing(12),
                )
                .primary_action(
                    widget::button::suggested(fl!("pack")).on_press(Message::DialogSubmit),
                )
                .secondary_action(cancel)
                .into()
        }
        Dialog::Connect {
            url,
            password,
            saved,
            found,
            browsing,
            note,
            ..
        } => {
            let mut col = column![
                widget::text::caption(fl!("connect-url")),
                field(url),
                widget::text::caption(fl!("connect-password")),
                widget::secure_input("", password.as_str(), None, true)
                    .on_input(Message::ConnectPassword)
                    .on_submit(|_| Message::DialogSubmit),
            ]
            .spacing(4);
            let pick = |label: String, url: &str| {
                widget::button::text(label)
                    .on_press(Message::ConnectPick(url.to_string()))
                    .width(Length::Fill)
            };
            if !saved.is_empty() {
                col = col.push(widget::text::caption(fl!("connect-saved")));
                for s in saved {
                    col = col.push(
                        row![
                            pick(s.clone(), s),
                            widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                                .on_press(Message::ConnectForget(s.clone())),
                        ]
                        .align_y(cosmic::iced::Alignment::Center),
                    );
                }
            }
            let browse = widget::button::standard(if *browsing {
                fl!("connect-browsing")
            } else {
                fl!("connect-browse")
            })
            .on_press_maybe((!browsing).then_some(Message::ConnectBrowse));
            col = col.push(browse);
            for (name, u) in found {
                col = col.push(pick(format!("{name}   {u}"), u));
            }
            if let Some((error, text)) = note {
                let t = widget::text::body(text.clone()).wrapping(Wrapping::WordOrGlyph);
                col = col.push(if *error {
                    t.class(cosmic::theme::Text::Custom(|t| {
                        cosmic::iced::widget::text::Style {
                            color: Some(t.cosmic().destructive_text_color().into()),
                            ..Default::default()
                        }
                    }))
                } else {
                    t
                });
            }
            widget::dialog()
                .title(fl!("connect"))
                .control(widget::scrollable(col).height(Length::Shrink))
                .primary_action(
                    widget::button::suggested(fl!("connect-go")).on_press(Message::DialogSubmit),
                )
                .secondary_action(cancel)
                .into()
        }
        Dialog::Unpack {
            archives,
            path,
            own_dir,
            ..
        } => widget::dialog()
            .title(fl!("unpack-to", what = what(archives)))
            .control(
                column![
                    field(path),
                    widget::checkbox(*own_dir)
                        .label(fl!("unpack-own-dir"))
                        .on_toggle(|_| Message::Toggle(Toggle::OwnDir)),
                ]
                .spacing(12),
            )
            .primary_action(
                widget::button::suggested(fl!("unpack")).on_press(Message::DialogSubmit),
            )
            .secondary_action(cancel)
            .into(),
        Dialog::Find(f) => {
            let edit = |label: String, value: &'a str, field: FindField| {
                let input = widget::text_input("", value)
                    .on_input(move |s| Message::FindInput(field, s))
                    .on_submit(|_| Message::FindSubmit);
                let input = if field == FindField::Mask {
                    input.id(input_id.clone())
                } else {
                    input
                };
                column![widget::text::caption(label), input].spacing(2)
            };
            let mut list = column![].spacing(2);
            for (i, p) in f.results.iter().enumerate().take(FIND_SHOWN) {
                let label = p.display().to_string();
                let b = if i == f.cursor {
                    widget::button::suggested(label)
                } else {
                    widget::button::text(label)
                };
                list = list.push(b.on_press(Message::FindPick(i)).width(Length::Fill));
            }
            let more = f.total.saturating_sub(f.results.len().min(FIND_SHOWN));
            if more > 0 {
                list = list.push(widget::text(fl!("find-more", n = more)));
            }
            let status = match (&f.error, &f.stop) {
                (Some(e), _) => e.clone(),
                (None, Some(_)) => format!("{}   {}", fl!("find-count", n = f.total), f.current),
                (None, None) => fl!("find-count", n = f.total),
            };
            let run = match &f.stop {
                Some(_) => widget::button::standard(fl!("find-stop")).on_press(Message::FindStop),
                None => widget::button::suggested(fl!("find-start")).on_press(Message::FindStart),
            };
            // Above the list, not in the dialog's bottom row: a short window clips that row.
            let go = widget::button::standard(fl!("find-go"))
                .on_press_maybe((!f.results.is_empty()).then_some(Message::FindPick(f.cursor)));
            let feed = widget::button::standard(fl!("find-feed"))
                .on_press_maybe((!f.results.is_empty()).then_some(Message::FindFeed));
            widget::dialog()
                .title(fl!("find-files"))
                .width(Length::Fill)
                .max_width(1100.0)
                .control(
                    column![
                        row![
                            edit(
                                if f.name_regex {
                                    fl!("find-name-regex-label")
                                } else {
                                    fl!("find-mask")
                                },
                                &f.mask,
                                FindField::Mask
                            ),
                            edit(fl!("find-in"), &f.dir, FindField::Dir),
                        ]
                        .spacing(8),
                        edit(fl!("find-text"), &f.text, FindField::Text),
                        // One row of four: every line here is taken from the result list.
                        widget::flex_row(vec![
                            widget::checkbox(f.case_sensitive)
                                .label(fl!("find-case"))
                                .on_toggle(|_| Message::FindCase)
                                .into(),
                            widget::checkbox(f.regex)
                                .label(fl!("find-regex"))
                                .on_toggle(|_| Message::FindRegex)
                                .into(),
                            widget::checkbox(f.name_regex)
                                .label(fl!("find-name-regex"))
                                .on_toggle(|_| Message::FindNameRegex)
                                .into(),
                            widget::checkbox(f.archives)
                                .label(fl!("find-archives"))
                                .on_toggle(|_| Message::FindArchives)
                                .into(),
                        ])
                        .spacing(16),
                        row![
                            edit(fl!("find-min-size"), &f.min_size, FindField::MinSize),
                            edit(fl!("find-max-size"), &f.max_size, FindField::MaxSize),
                            edit(fl!("find-days"), &f.days, FindField::Days),
                        ]
                        .spacing(8),
                        widget::flex_row(vec![run.into(), go.into(), feed.into(), cancel.into()])
                            .spacing(8),
                        widget::text(status).wrapping(Wrapping::WordOrGlyph),
                        widget::scrollable(list).height(list_height(f.results.len())),
                    ]
                    .spacing(12),
                )
                .into()
        }
        Dialog::Diff(d) => {
            let name = |p: &PathBuf| p.display().to_string();
            let mut list = column![];
            let status = match d.result.as_deref() {
                None => fl!("diff-running"),
                Some(Err(e)) => e.clone(),
                Some(Ok(diff::Outcome::Binary { same: true })) => fl!("diff-binary-same"),
                Some(Ok(diff::Outcome::Binary { same: false })) => fl!("diff-binary-differ"),
                Some(Ok(diff::Outcome::Text(t))) => {
                    let mono = |s: String| {
                        widget::text(s)
                            .font(cosmic::font::mono())
                            .wrapping(Wrapping::None)
                    };
                    // `mid`: the bytes that differ from the other side (changed lines).
                    let half = |c: &Option<(usize, String)>, w: f32, mid: Option<Range<usize>>| {
                        let content = match (c, mid) {
                            (Some((n, s)), Some(m)) => row![
                                mono(format!("{n:>5} ")).width(Length::Fixed(NUM_W)),
                                mono(expand(&s[..m.start])),
                                widget::container(mono(expand(&s[m.clone()])))
                                    .class(diff_style(diff::Kind::Changed)),
                                mono(expand(&s[m.end..])),
                            ],
                            (Some((n, s)), None) => row![
                                mono(format!("{n:>5} ")).width(Length::Fixed(NUM_W)),
                                mono(expand(s)),
                            ],
                            (None, _) => row![],
                        };
                        widget::container(content)
                            .width(Length::Fixed(w))
                            .clip(true)
                    };
                    // Only the rows in view (plus a margin) are built; spacers keep the height.
                    let first = ((d.offset / DIFF_ROW_H) as usize).min(t.rows.len());
                    let last = (first + (DIFF_LIST_H / DIFF_ROW_H) as usize + 2).min(t.rows.len());
                    let (wl, wr) = d.widths;
                    list = list.push(
                        widget::Space::new()
                            .width(Length::Fixed(wl + wr + 8.0))
                            .height(Length::Fixed(first as f32 * DIFF_ROW_H)),
                    );
                    for r in &t.rows[first..last] {
                        let (ml, mr) = match (&r.left, &r.right) {
                            (Some((_, a)), Some((_, b))) if r.kind == diff::Kind::Changed => {
                                let (x, y) = diff::inline(a, b);
                                (Some(x), Some(y))
                            }
                            _ => (None, None),
                        };
                        list = list.push(
                            widget::container(
                                row![half(&r.left, wl, ml), half(&r.right, wr, mr)].spacing(8),
                            )
                            .height(Length::Fixed(DIFF_ROW_H))
                            .class(diff_style(r.kind)),
                        );
                    }
                    let below = (t.rows.len() - last) as f32 * DIFF_ROW_H;
                    list = list.push(
                        widget::Space::new()
                            .width(Length::Fixed(1.0))
                            .height(Length::Fixed(below)),
                    );
                    let more = t.total.saturating_sub(t.rows.len());
                    if more > 0 {
                        list = list.push(widget::text(fl!("find-more", n = more)));
                    }
                    match t.blocks.len() {
                        0 if t.eol_differs => fl!("diff-eol"),
                        0 => fl!("diff-same"),
                        n => fl!(
                            "diff-count",
                            i = (d.block + 1).to_string(),
                            n = n.to_string()
                        ),
                    }
                }
            };
            let nav = !d.blocks().is_empty();
            let dirty = d.dirty.0 || d.dirty.1;
            let status = match (dirty, d.confirm_close) {
                (true, true) => format!("{status}   {}", fl!("diff-unsaved-close")),
                (true, false) => format!("{status}   {}", fl!("diff-unsaved")),
                _ => status,
            };
            let buttons = widget::flex_row(vec![
                widget::button::standard(fl!("diff-prev"))
                    .on_press_maybe(nav.then_some(Message::DiffPrev))
                    .into(),
                widget::button::standard(fl!("diff-next"))
                    .on_press_maybe(nav.then_some(Message::DiffNext))
                    .into(),
                widget::button::standard(fl!("diff-copy-right"))
                    .on_press_maybe(nav.then_some(Message::DiffCopy(true)))
                    .into(),
                widget::button::standard(fl!("diff-copy-left"))
                    .on_press_maybe(nav.then_some(Message::DiffCopy(false)))
                    .into(),
                widget::button::suggested(fl!("diff-save"))
                    .on_press_maybe((dirty && d.text().is_some()).then_some(Message::DiffSave))
                    .into(),
                cancel.into(),
            ])
            .spacing(8);
            let opts = row![
                widget::checkbox(d.opts.ignore_space)
                    .label(fl!("diff-ignore-space"))
                    .on_toggle(|_| Message::DiffOpt(DiffOpt::Space)),
                widget::checkbox(d.opts.ignore_case)
                    .label(fl!("diff-ignore-case"))
                    .on_toggle(|_| Message::DiffOpt(DiffOpt::Case)),
            ]
            .spacing(16);
            widget::dialog()
                .title(fl!("diff-title"))
                .width(Length::Fill)
                .max_width(1400.0)
                .control(
                    column![
                        row![
                            widget::text(name(&d.left))
                                .width(Length::FillPortion(1))
                                .wrapping(Wrapping::WordOrGlyph),
                            widget::text(name(&d.right))
                                .width(Length::FillPortion(1))
                                .wrapping(Wrapping::WordOrGlyph),
                        ]
                        .spacing(8),
                        // All buttons above the text: a short window clips the dialog's bottom row.
                        opts,
                        buttons,
                        widget::text(status),
                        widget::scrollable(list)
                            .id(d.scroll.clone())
                            .on_scroll(|v| Message::DiffScrolled(v.absolute_offset().y))
                            .direction(cosmic::iced::widget::scrollable::Direction::Both {
                                vertical: Default::default(),
                                horizontal: Default::default(),
                            })
                            .width(Length::Fill)
                            .height(Length::Fixed(DIFF_LIST_H)),
                    ]
                    .spacing(10),
                )
                .into()
        }
        Dialog::Sync(s) => {
            use sync::Kind;
            const W_SIZE: f32 = 100.0;
            const W_DATE: f32 = 130.0;
            const W_ACT: f32 = 44.0;
            // Copy → / ← in the accent colour, ✕ red, ≠ and = plain (TC colours rows the same way).
            let tint = |k: Kind| -> cosmic::theme::Text {
                match k {
                    Kind::ToRight | Kind::ToLeft => cosmic::theme::Text::Accent,
                    Kind::Delete => {
                        cosmic::theme::Text::Custom(|t| cosmic::iced::widget::text::Style {
                            color: Some(t.cosmic().destructive_text_color().into()),
                            ..Default::default()
                        })
                    }
                    _ => cosmic::theme::Text::Default,
                }
            };
            let cell = |t: String, k: Kind| {
                widget::text(t)
                    .size(13)
                    .class(tint(k))
                    .wrapping(Wrapping::None)
            };
            // One side: name (path below the compared dirs), size, date — mirrored on the right.
            let side = |i: &Option<sync::Info>, rel: &str, k: Kind, left: bool| {
                let (name, size, date) = match i {
                    None => (String::new(), String::new(), String::new()),
                    Some(i) => (
                        rel.to_string(),
                        if i.dir {
                            "<DIR>".into()
                        } else {
                            format::size(i.size)
                        },
                        format::date(i.mtime, tz),
                    ),
                };
                let name = cell(name, k).width(Length::Fill);
                let size = cell(size, k)
                    .width(Length::Fixed(W_SIZE))
                    .align_x(cosmic::iced::Alignment::End);
                let date = cell(date, k).width(Length::Fixed(W_DATE));
                if left {
                    row![name, size, date].spacing(8)
                } else {
                    row![date, size, name].spacing(8)
                }
            };
            let c = sync::counts(&s.rows);
            let mut list = column![].spacing(2);
            let mut shown = 0;
            for (i, r) in s.rows.iter().enumerate() {
                let k = r.kind();
                // ✕ rows are always listed: nothing is deleted unseen.
                if k != Kind::Delete && (s.hide.contains(&k) || shown >= FIND_SHOWN) {
                    continue;
                }
                shown += 1;
                let arrow = match k {
                    Kind::ToRight => "→",
                    Kind::ToLeft => "←",
                    Kind::Delete => "✕",
                    Kind::Same => "=",
                    Kind::Differ => "≠",
                };
                let rel = r.rel.display().to_string();
                list = list.push(
                    row![
                        side(&r.left, &rel, k, true).width(Length::FillPortion(1)),
                        // Custom: the standard button has a minimum height; rows stay dense, as in TC.
                        widget::button::custom(
                            widget::text(arrow)
                                .width(Length::Fill)
                                .align_x(cosmic::iced::Alignment::Center),
                        )
                        .class(cosmic::theme::Button::Standard)
                        .padding([1, 4])
                        .width(Length::Fixed(W_ACT))
                        .on_press(Message::SyncFlip(i)),
                        side(&r.right, &rel, k, false).width(Length::FillPortion(1)),
                    ]
                    .spacing(8)
                    .align_y(cosmic::iced::Alignment::Center),
                );
            }
            let head = |left: bool| {
                let h = |t: String| widget::text::caption(t);
                let (name, size, date) = (
                    h(fl!("col-name")).width(Length::Fill),
                    h(fl!("col-size"))
                        .width(Length::Fixed(W_SIZE))
                        .align_x(cosmic::iced::Alignment::End),
                    h(fl!("col-date")).width(Length::Fixed(W_DATE)),
                );
                if left {
                    row![name, size, date].spacing(8)
                } else {
                    row![date, size, name].spacing(8)
                }
                .width(Length::FillPortion(1))
            };
            let header = row![
                head(true),
                widget::Space::new().width(Length::Fixed(W_ACT)),
                head(false)
            ]
            .spacing(8);
            let opt = |label: String, on: bool, o: SyncOpt| {
                widget::checkbox(on)
                    .label(label)
                    .on_toggle(move |_| Message::SyncOpt(o))
            };
            // TC's show buttons, with what each means and how many there are.
            let show = |k: Kind, label: String| {
                let b = if s.hide.contains(&k) {
                    widget::button::standard(label)
                } else {
                    widget::button::suggested(label)
                };
                b.on_press(Message::SyncShow(k))
            };
            let compare = match &s.running {
                Some(_) => widget::button::standard(fl!("find-stop")).on_press(Message::SyncStop),
                None => {
                    widget::button::standard(fl!("sync-compare")).on_press(Message::SyncCompare)
                }
            };
            let todo = c.to_right + c.to_left + c.delete;
            let run = if s.confirm {
                widget::button::destructive(fl!("sync-run-delete"))
            } else {
                widget::button::suggested(fl!("sync-run"))
            }
            .on_press_maybe((todo > 0 && s.running.is_none()).then_some(Message::SyncRun));
            let mut note = match (c.delete, s.confirm) {
                (0, _) => String::new(),
                (x, false) => fl!("sync-to-delete", n = x),
                (x, true) => fl!("sync-confirm-delete", n = x),
            };
            if s.running.is_some() {
                note = fl!("sync-comparing");
            }
            let path = |p: &std::path::Path| {
                widget::text::heading(p.display().to_string())
                    .wrapping(Wrapping::WordOrGlyph)
                    .width(Length::FillPortion(1))
            };
            widget::dialog()
                .title(fl!("sync-dirs"))
                .width(Length::Fill)
                .max_width(1200.0)
                .control(
                    column![
                        row![
                            path(&s.left),
                            widget::Space::new().width(Length::Fixed(W_ACT)),
                            path(&s.right)
                        ]
                        .spacing(8),
                        row![
                            widget::text_input(fl!("sync-mask"), &s.mask)
                                .on_input(Message::SyncMask)
                                .on_submit(|_| Message::SyncCompare)
                                .width(Length::Fixed(200.0)),
                            opt(fl!("sync-subdirs"), s.recursive, SyncOpt::Recursive),
                            opt(fl!("sync-content"), s.content, SyncOpt::Content),
                            opt(fl!("sync-ignore-date"), s.ignore_date, SyncOpt::IgnoreDate),
                            opt(fl!("sync-mirror"), s.mirror, SyncOpt::Mirror),
                        ]
                        .spacing(16)
                        .align_y(cosmic::iced::Alignment::Center),
                        // All buttons above the list: a short window clips the dialog's bottom row.
                        row![
                            widget::text::body(fl!("sync-show")),
                            show(Kind::ToRight, fl!("sync-show-right", n = c.to_right)),
                            show(Kind::ToLeft, fl!("sync-show-left", n = c.to_left)),
                            show(Kind::Differ, fl!("sync-show-differ", n = c.differ)),
                            show(Kind::Same, fl!("sync-show-same", n = c.same)),
                            widget::Space::new().width(Length::Fill),
                            compare,
                            run,
                            cancel,
                        ]
                        .spacing(8)
                        .align_y(cosmic::iced::Alignment::Center),
                        header,
                        widget::scrollable(list).height(list_height(shown)),
                        widget::text::body(note),
                    ]
                    .spacing(10),
                )
                .into()
        }
        Dialog::Error { path, error, .. } => widget::dialog()
            .title(fl!("op-error"))
            .body(format!("{}\n{error}", path.display()))
            .primary_action(
                widget::button::suggested(fl!("retry"))
                    .on_press(Message::ErrorAnswer(ErrorChoice::Retry)),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press(Message::ErrorAnswer(ErrorChoice::Cancel)),
            )
            .tertiary_action(
                widget::button::standard(fl!("skip"))
                    .on_press(Message::ErrorAnswer(ErrorChoice::Skip)),
            )
            .into(),
    }
}

/// "Packing", "Copying"…: the job's title, in its dialog and in the status line.
pub fn job_title(kind: OpKind) -> String {
    match kind {
        OpKind::Copy => fl!("copying"),
        OpKind::Move => fl!("moving"),
        OpKind::Delete => fl!("deleting"),
        OpKind::Pack => fl!("packing"),
        OpKind::Unpack => fl!("unpacking"),
        OpKind::Extract => fl!("extracting"),
        OpKind::Sync => fl!("syncing"),
        OpKind::Repack => fl!("repacking"),
    }
}

/// Share done, 0..=1 (0 while the total is not known yet).
pub fn job_fraction(job: &Running) -> f32 {
    job.done as f32 / job.total.max(1) as f32
}

/// Shown while a job runs, its window is not hidden and no question is pending.
pub fn progress(job: &Running) -> Element<'_, Message> {
    let counts = match job.kind {
        OpKind::Delete => fl!(
            "progress-items",
            done = job.done.to_string(),
            total = job.total.to_string()
        ),
        _ => fl!(
            "progress-bytes",
            done = format::size(job.done),
            total = format::size(job.total)
        ),
    };
    let fraction = job_fraction(job);
    let percent = format!("{} %", (fraction * 100.0).round() as u32);
    widget::dialog()
        .title(job_title(job.kind))
        .body(job.current.clone())
        .control(
            column![
                widget::progress_bar::determinate_linear(fraction)
                    .width(Length::Fill)
                    .girth(12),
                row![
                    widget::text(counts).width(Length::Fill),
                    widget::text::heading(percent)
                ],
            ]
            .spacing(8)
            .width(Length::Fill),
        )
        // TC "Background": the panels work again, the job goes on (shown in the status line).
        .primary_action(widget::button::standard(fl!("cancel")).on_press(Message::CancelJob))
        .secondary_action(widget::button::suggested(fl!("job-hide")).on_press(Message::JobHide))
        .into()
}
