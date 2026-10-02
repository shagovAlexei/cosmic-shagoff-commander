//! Modal dialogs: one enum, one view per variant.

use crate::app::{Message, OpKind, Running};
use crate::fl;
use cosmic::iced::widget::{column, row};
use cosmic::{Element, widget};
use shagoff_core::drives::Drive;
use shagoff_core::format::{self, TimeZone};
use shagoff_core::ops::{ErrorChoice, FileInfo, Resolution};
use std::path::PathBuf;
use std::sync::mpsc;

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
    /// Alt+F1 / Alt+F2.
    Drives { side: usize, cursor: usize },
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

pub fn view<'a>(
    d: &'a Dialog,
    input_id: &widget::Id,
    tz: &TimeZone,
    drives: &'a [Drive],
) -> Element<'a, Message> {
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
        Dialog::Drives { side, cursor } => {
            let mut list = column![].spacing(2);
            for (i, d) in drives.iter().enumerate() {
                let label = format!("{}   {}", d.label, d.path.display());
                let b = if i == *cursor {
                    widget::button::suggested(label)
                } else {
                    widget::button::text(label)
                };
                list = list.push(
                    b.on_press(Message::Drive(*side, i))
                        .width(cosmic::iced::Length::Fill),
                );
            }
            widget::dialog()
                .title(fl!("drives"))
                .control(list)
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
