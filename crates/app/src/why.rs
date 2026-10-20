//! An error as the user reads it: core's refusals (`shagoff_core::why`) and the common system
//! errors in the UI's language; anything else as the system says it.

use crate::fl;
use std::io::{self, ErrorKind};

/// `e` for the user. The "password needed" text stays as core wrote it: that is how the app
/// knows to ask for one (`archive::needs_password`); `message` translates it where it is shown.
pub fn text(e: &io::Error) -> String {
    let raw = e.to_string();
    if shagoff_core::archive::needs_password(&raw) {
        return raw;
    }
    // A system error, or one core made of a kind alone: by its kind (the system's own text is
    // English, "… (os error 13)").
    if e.raw_os_error().is_some() || e.get_ref().is_none() {
        let t = match (e.raw_os_error(), e.kind()) {
            // EPERM is not EACCES: an immutable file, a chown not allowed — not the mode bits.
            (Some(1), _) => Some(fl!("err-not-permitted")),
            (_, ErrorKind::PermissionDenied) => Some(fl!("err-permission")),
            (_, ErrorKind::NotFound) => Some(fl!("err-not-found")),
            (_, ErrorKind::AlreadyExists) => Some(fl!("err-exists")),
            (_, ErrorKind::StorageFull) => Some(fl!("err-full")),
            (_, ErrorKind::QuotaExceeded) => Some(fl!("err-quota")),
            (_, ErrorKind::ReadOnlyFilesystem) => Some(fl!("err-read-only")),
            (_, ErrorKind::DirectoryNotEmpty) => Some(fl!("err-not-empty")),
            (_, ErrorKind::NotADirectory) => Some(fl!("err-not-dir")),
            (_, ErrorKind::IsADirectory) => Some(fl!("err-is-dir")),
            (_, ErrorKind::InvalidFilename) => Some(fl!("err-bad-name")),
            (_, ErrorKind::ResourceBusy) => Some(fl!("err-busy")),
            (_, ErrorKind::TimedOut) => Some(fl!("err-timeout")),
            (_, ErrorKind::CrossesDevices) => Some(fl!("err-cross-device")),
            _ => None,
        };
        if let Some(t) = t {
            return t;
        }
    }
    message(&raw)
}

/// A text from core: translated if it is one of its refusals.
pub fn message(s: &str) -> String {
    match shagoff_core::why::ALL.iter().find(|(_, t)| *t == s) {
        Some((key, _)) => crate::i18n::LANGUAGE_LOADER.get(key),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_job_errors_in_the_ui_language() {
        let ru = include_str!("../i18n/ru/shagoff-commander.ftl");
        for (key, text) in shagoff_core::why::ALL {
            assert!(crate::i18n::LANGUAGE_LOADER.has(key), "en: {key}");
            assert!(
                ru.lines().any(|l| l.starts_with(&format!("{key} ="))),
                "ru: {key}"
            );
            assert_ne!(message(text), *text, "{key}");
        }
        assert_eq!(message("something else"), "something else");
        assert_eq!(
            text(&io::Error::from_raw_os_error(13)),
            fl!("err-permission")
        );
        assert_eq!(
            text(&io::Error::from_raw_os_error(1)),
            fl!("err-not-permitted")
        );
        // Made by core of a kind alone: translated too.
        assert_eq!(
            text(&io::Error::from(ErrorKind::AlreadyExists)),
            fl!("err-exists")
        );
        let own = io::Error::other(shagoff_core::why::SPECIAL);
        assert_eq!(text(&own), fl!("why-special"));
        let odd = io::Error::from_raw_os_error(71); // EPROTO: no text of ours
        assert_eq!(text(&odd), odd.to_string());
        // The password one: kept (the prompt is told by it), translated where shown.
        let pw = io::Error::other(shagoff_core::why::PASSWORD);
        assert!(shagoff_core::archive::needs_password(&text(&pw)));
        assert_eq!(message(&text(&pw)), fl!("why-password"));
    }
}
