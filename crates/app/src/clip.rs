//! System clipboard glue: `shagoff_core::clipboard` payloads ↔ iced mime traits.

use crate::app::Message;
use cosmic::app::Task;
use cosmic::iced::clipboard::{
    self,
    mime::{AllowedMimeTypes, AsMimeTypes},
};
use shagoff_core::clipboard::{self as fmt, Kind, Mime};
use std::borrow::Cow;
use std::path::PathBuf;

struct Files(Mime);

impl AsMimeTypes for Files {
    fn available(&self) -> Cow<'static, [String]> {
        Cow::Owned(
            [
                "text/plain",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                fmt::URI_LIST,
                fmt::GNOME,
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        let s = match mime_type {
            "text/plain" | "text/plain;charset=utf-8" | "UTF8_STRING" => &self.0.plain,
            fmt::URI_LIST => &self.0.uri_list,
            fmt::GNOME => &self.0.gnome,
            _ => return None,
        };
        Some(Cow::Owned(s.clone().into_bytes()))
    }
}

struct Paste(Kind, Vec<PathBuf>);

impl AllowedMimeTypes for Paste {
    fn allowed() -> Cow<'static, [String]> {
        Cow::Owned(vec![fmt::GNOME.into(), fmt::URI_LIST.into()])
    }
}

impl TryFrom<(Vec<u8>, String)> for Paste {
    type Error = ();
    fn try_from((data, mime): (Vec<u8>, String)) -> Result<Self, ()> {
        fmt::decode(&mime, &data)
            .map(|(k, p)| Paste(k, p))
            .ok_or(())
    }
}

/// Ctrl+C / Ctrl+X.
pub fn put<M>(kind: Kind, paths: &[PathBuf]) -> Task<M> {
    clipboard::write_data(Files(fmt::encode(kind, paths)))
}

/// After a cut is pasted, so a second Ctrl+V does not try to move files that are gone.
pub fn clear<M>() -> Task<M> {
    clipboard::write(String::new())
}

/// Ctrl+V: files on the clipboard, if any, arrive as `Message::Pasted`.
pub fn take() -> Task<Message> {
    clipboard::read_data::<Paste>()
        .map(|p| cosmic::Action::App(Message::Pasted(p.map(|Paste(k, v)| (k, v)))))
}

/// A row that can be dragged out: the files go as on Ctrl+C (uri-list + gnome), so Files and the
/// desktop take them too. The source is the window: a widget id is new on every frame, so the
/// runtime would not find the row it started from. Copy only: offered Move too, the desktop moved
/// the files away (Shift-move between our own panes reads `App::mods`, not the action).
pub fn drag<'a>(
    child: impl Into<cosmic::Element<'a, Message>>,
    paths: std::sync::Arc<Vec<PathBuf>>,
    window: cosmic::iced::window::Id,
) -> cosmic::Element<'a, Message> {
    use cosmic::iced::clipboard::dnd::DndAction;
    use cosmic::widget;
    widget::dnd_source(child)
        .action(DndAction::Copy)
        .drag_content(move || Files(fmt::encode(Kind::Copy, &paths)))
        .window(window)
        .on_finish(Some(Message::DropHover(None)))
        .on_cancel(Some(Message::DropHover(None)))
        .into()
}

/// Files dropped on `child` (from us or another program) arrive as `on(paths, move?)`.
/// Copy unless the compositor picked Move (Shift held).
pub fn drop_zone<'a>(
    child: impl Into<cosmic::Element<'a, Message>>,
    on: impl Fn(Vec<PathBuf>, bool) -> Message + 'static,
    hover: Option<(usize, usize)>,
) -> cosmic::Element<'a, Message> {
    use cosmic::iced::clipboard::dnd::DndAction;
    let mut d = cosmic::widget::dnd_destination::DndDestination::for_data::<Paste>(
        child,
        move |p, action| {
            on(
                p.map(|Paste(_, v)| v).unwrap_or_default(),
                action == DndAction::Move,
            )
        },
    )
    .preferred_action(DndAction::Copy);
    // A row lights up while files are held over it; leaving clears only its own light (the next
    // row's enter may come first).
    if let Some(h) = hover {
        d = d
            .on_enter(move |_, _, _| Message::DropHover(Some(h)))
            .on_leave(move || Message::DropLeave(h));
    }
    d.into()
}
