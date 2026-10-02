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
