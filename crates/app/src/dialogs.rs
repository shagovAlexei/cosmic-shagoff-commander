//! Modal dialogs: one enum, one view per variant.

use crate::app::Message;
use crate::fl;
use cosmic::{Element, widget};
use std::path::PathBuf;

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
}

impl Dialog {
    /// The text field, for dialogs that have one.
    pub fn input_mut(&mut self) -> Option<&mut String> {
        match self {
            Dialog::Mask { input, .. } | Dialog::Input { input, .. } => Some(input),
            Dialog::ConfirmDelete { .. } => None,
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

pub fn view<'a>(d: &'a Dialog, input_id: &widget::Id) -> Element<'a, Message> {
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
    }
}
