//! An error as the user reads it: core's refusals (`shagoff_core::why`) and the common system
//! errors in the UI's language; anything else as the system says it.

use crate::fl;
use shagoff_core::why as core;
use std::io::{self, ErrorKind};

/// `e` for the user.
pub fn text(e: &io::Error) -> String {
    // A system error: by its kind (its own text is English, "… (os error 13)").
    if e.raw_os_error().is_some() {
        let t = match e.kind() {
            ErrorKind::PermissionDenied => Some(fl!("err-permission")),
            ErrorKind::NotFound => Some(fl!("err-not-found")),
            ErrorKind::AlreadyExists => Some(fl!("err-exists")),
            ErrorKind::StorageFull => Some(fl!("err-full")),
            ErrorKind::ReadOnlyFilesystem => Some(fl!("err-read-only")),
            ErrorKind::DirectoryNotEmpty => Some(fl!("err-not-empty")),
            ErrorKind::NotADirectory => Some(fl!("err-not-dir")),
            ErrorKind::IsADirectory => Some(fl!("err-is-dir")),
            ErrorKind::InvalidFilename => Some(fl!("err-bad-name")),
            ErrorKind::ResourceBusy => Some(fl!("err-busy")),
            ErrorKind::TimedOut => Some(fl!("err-timeout")),
            _ => None,
        };
        if let Some(t) = t {
            return t;
        }
    }
    message(&e.to_string())
}

/// A text from core: translated if it is one of its refusals.
pub fn message(s: &str) -> String {
    match s {
        core::SPECIAL => fl!("why-special"),
        core::INTO_ITSELF => fl!("why-into-itself"),
        core::OTHER_TYPE => fl!("why-other-type"),
        core::UNSAFE_PATH => fl!("why-unsafe-path"),
        core::THROUGH_LINK | core::INNER_LINK => fl!("why-through-link"),
        core::DIR_TAKEN => fl!("why-dir-taken"),
        core::HARDLINK_OUT => fl!("why-hardlink-out"),
        core::NO_END_MARKER => fl!("why-no-end-marker"),
        core::NOT_PACK_FORMAT => fl!("why-not-pack-format"),
        core::REPLACES_SOURCE => fl!("why-replaces-source"),
        core::NOT_UTF8 => fl!("why-not-utf8"),
        core::NOT_CHANGEABLE => fl!("why-not-changeable"),
        core::LEGACY_NAMES => fl!("why-legacy-names"),
        core::BAD_INNER_PATH => fl!("why-unsafe-path"),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_reason_is_translated_and_system_errors_by_kind() {
        for s in core::ALL {
            assert_ne!(message(s), s, "{s}");
        }
        assert_eq!(message("something else"), "something else");
        let denied = io::Error::from_raw_os_error(13); // EACCES
        assert_eq!(text(&denied), fl!("err-permission"));
        let own = io::Error::other(core::SPECIAL);
        assert_eq!(text(&own), fl!("why-special"));
        let odd = io::Error::from_raw_os_error(71); // EPROTO: no text of ours
        assert_eq!(text(&odd), odd.to_string());
    }
}
