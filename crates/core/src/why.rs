//! Why core refuses something: the texts its errors carry (in English, as `io::Error`s). One place
//! for them, so the UI can show each in the user's language (`app::why`).

pub const SPECIAL: &str = "special file (pipe, socket or device): not copied";
pub const INTO_ITSELF: &str = "cannot copy a folder into itself";
pub const OTHER_TYPE: &str = "an entry of another type has this name";
pub const UNSAFE_PATH: &str = "unsafe path in archive";
pub const THROUGH_LINK: &str = "path goes through a link in the archive";
pub const DIR_TAKEN: &str = "a folder with this name is already in the archive";
pub const HARDLINK_OUT: &str = "hard link to a file outside the archive";
pub const NO_END_MARKER: &str = "the tar ends without its end marker";
pub const NOT_PACK_FORMAT: &str = "not a packing format";
pub const REPLACES_SOURCE: &str = "the archive would replace a file being packed";
pub const NOT_UTF8: &str = "name is not UTF-8";
pub const NOT_CHANGEABLE: &str = "not an archive that can be changed";
pub const LEGACY_NAMES: &str =
    "file names in a legacy encoding; changing the archive would garble them";
pub const BAD_INNER_PATH: &str = "bad path inside the archive";
pub const INNER_LINK: &str = "the path goes through a link inside the archive";

/// Every text above (the UI checks it knows them all).
pub const ALL: [&str; 15] = [
    SPECIAL,
    INTO_ITSELF,
    OTHER_TYPE,
    UNSAFE_PATH,
    THROUGH_LINK,
    DIR_TAKEN,
    HARDLINK_OUT,
    NO_END_MARKER,
    NOT_PACK_FORMAT,
    REPLACES_SOURCE,
    NOT_UTF8,
    NOT_CHANGEABLE,
    LEGACY_NAMES,
    BAD_INNER_PATH,
    INNER_LINK,
];
