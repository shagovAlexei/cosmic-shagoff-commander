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

/// Files as the clipboard and other programs take them; `inner`: only as our own type (entries
/// inside an archive — not on disk, so no other program may take them).
struct Files(Mime, bool);

impl AsMimeTypes for Files {
    fn available(&self) -> Cow<'static, [String]> {
        if self.1 {
            return Cow::Owned(vec![fmt::INNER.into()]);
        }
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
            fmt::INNER if self.1 => &self.0.uri_list,
            _ if self.1 => return None,
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
        Cow::Owned(vec![
            fmt::INNER.into(),
            fmt::GNOME.into(),
            fmt::URI_LIST.into(),
        ])
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
    clipboard::write_data(Files(fmt::encode(kind, paths), false))
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
/// The right button too (`crate::dnd`): `Message::DragStart` remembers that drag's files, so their
/// drop on us asks Copy / Move / Link.
/// `icon`: what follows the pointer (`drag_card`), built when the drag starts. `inner`: entries
/// inside an archive, offered only as our own type (a drop on us copies them out, as F5).
pub fn drag<'a>(
    child: impl Into<cosmic::Element<'a, Message>>,
    paths: std::sync::Arc<Vec<PathBuf>>,
    inner: bool,
    window: cosmic::iced::window::Id,
    icon: impl Fn() -> cosmic::Element<'static, ()> + 'a,
) -> cosmic::Element<'a, Message> {
    use cosmic::iced::clipboard::dnd::DndAction;
    let started = paths.clone();
    crate::dnd::drag_source(
        child,
        window,
        DndAction::Copy,
        move || Files(fmt::encode(Kind::Copy, &paths), inner),
        icon,
        move |right| Message::DragStart(right.then(|| started.clone())),
        Message::DragEnd,
    )
    .into()
}

/// The picture under the pointer: an icon and the file's name (or how many), on a card; a long
/// name is cut, so the card stays small.
pub fn drag_card(icon: cosmic::widget::Icon, label: String) -> cosmic::Element<'static, ()> {
    use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
    use cosmic::widget;
    widget::container(
        cosmic::iced::widget::row![
            icon,
            widget::text::body(label)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
        ]
        .spacing(6)
        .align_y(cosmic::iced::Alignment::Center),
    )
    .padding([4, 10])
    .max_width(320.0)
    .class(cosmic::theme::Container::Card)
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
