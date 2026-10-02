//! Modal dialogs: one enum, one view per variant.

use crate::app::{Message, OpKind, Running};
use crate::fl;
use cosmic::iced::Length;
use cosmic::iced::widget::{column, row};
use cosmic::{Element, widget};
use shagoff_core::format::{self, TimeZone};
use shagoff_core::multirename::{self, Case, Counter, Problem, Row, Rule};
use shagoff_core::ops::{ErrorChoice, FileInfo, Resolution};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOp {
    Copy,
    Move,
    Mkdir,
    Rename,
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
    /// Ctrl+M: files snapshot at open (panel order) and the form. Boxed: the form is large.
    MultiRename(Box<MultiRename>),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub label: String,
    /// Empty for the hotlist's "add current dir" row.
    pub path: PathBuf,
}

impl Dialog {
    /// The text field, for dialogs that have one.
    pub fn input_mut(&mut self) -> Option<&mut String> {
        match self {
            Dialog::Mask { input, .. } | Dialog::Input { input, .. } => Some(input),
            _ => None,
        }
    }
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
            };
            widget::dialog()
                .title(title)
                .control(field(input))
                .primary_action(ok)
                .secondary_action(cancel)
                .into()
        }
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
        Dialog::Conflict { src, dst, .. } => {
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
        Dialog::List {
            kind,
            cursor,
            items,
            ..
        } => {
            let mut list = column![].spacing(2);
            for (i, item) in items.iter().enumerate() {
                let label = if item.path.as_os_str().is_empty() {
                    item.label.clone()
                } else {
                    format!("{}   {}", item.label, item.path.display())
                };
                let b = if i == *cursor {
                    widget::button::suggested(label)
                } else {
                    widget::button::text(label)
                };
                list = list.push(
                    b.on_press(Message::ListPick(i))
                        .width(cosmic::iced::Length::Fill),
                );
            }
            let title = match kind {
                ListKind::Drives => fl!("drives"),
                ListKind::History => fl!("history"),
                ListKind::Hotlist => fl!("hotlist"),
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
            let mut table = column![row![
                widget::text::heading(fl!("mr-old")).width(Length::FillPortion(1)),
                widget::text::heading(fl!("mr-new")).width(Length::FillPortion(1)),
            ]]
            .spacing(2);
            let rows = m.rows(tz);
            for r in &rows {
                let new = match r.problem {
                    None => r.new.clone(),
                    Some(p) => format!("⚠ {}  ({})", r.new, problem(p)),
                };
                table = table.push(row![
                    widget::text(r.old.clone()).width(Length::FillPortion(1)),
                    widget::text(new).width(Length::FillPortion(1)),
                ]);
            }
            let ok = rows.iter().all(|r| r.problem.is_none());
            widget::dialog()
                .title(fl!("multi-rename"))
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

/// Shown while a job runs and no question is pending.
pub fn progress(job: &Running) -> Element<'_, Message> {
    let title = match job.kind {
        OpKind::Copy => fl!("copying"),
        OpKind::Move => fl!("moving"),
        OpKind::Delete => fl!("deleting"),
    };
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
    let fraction = job.done as f32 / job.total.max(1) as f32;
    widget::dialog()
        .title(title)
        .body(job.current.clone())
        .control(
            column![
                widget::progress_bar::determinate_linear(fraction),
                widget::text(counts)
            ]
            .spacing(8),
        )
        .primary_action(widget::button::standard(fl!("cancel")).on_press(Message::CancelJob))
        .into()
}
