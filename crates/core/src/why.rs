//! Why core refuses something: the texts its errors carry (in English, as `io::Error`s), each with
//! the key of its translation in the app's `.ftl` (`app::why` shows them in the UI's language).
//! One macro line per reason: a constant, its key and its text can't drift apart.

macro_rules! reasons {
    ($($name:ident = $key:literal, $text:literal;)*) => {
        $(pub const $name: &str = $text;)*
        /// (translation key, text) of every reason.
        pub const ALL: &[(&str, &str)] = &[$(($key, $text)),*];
    };
}

reasons! {
    SPECIAL = "why-special", "special file (pipe, socket or device): not copied";
    INTO_ITSELF = "why-into-itself", "cannot copy a folder into itself";
    OTHER_TYPE = "why-other-type", "an entry of another type has this name";
    UNSAFE_PATH = "why-unsafe-path", "unsafe path in archive";
    THROUGH_LINK = "why-through-link", "path goes through a link in the archive";
    DIR_TAKEN = "why-dir-taken", "a folder with this name is already in the archive";
    HARDLINK_OUT = "why-hardlink-out", "hard link to a file outside the archive";
    NO_END_MARKER = "why-no-end-marker", "the tar ends without its end marker";
    NOT_PACK_FORMAT = "why-not-pack-format", "not a packing format";
    REPLACES_SOURCE = "why-replaces-source", "the archive would replace a file being packed";
    NOT_UTF8 = "why-not-utf8", "name is not UTF-8";
    NOT_CHANGEABLE = "why-not-changeable", "not an archive that can be changed";
    LEGACY_NAMES = "why-legacy-names",
        "file names in a legacy encoding; changing the archive would garble them";
    CANT_UNPACK = "why-cant-unpack",
        "some entries can't be unpacked; the archive is left as it was";
    CANT_REPACK = "why-cant-repack",
        "some entries can't be packed again; the archive is left as it was";
    PASSWORD = "why-password", "the archive's password is needed (or the one given is wrong)";
    READ_ONLY = "why-read-only", "the file is read-only";
    HARD_LINKS = "why-hard-links", "the file has other hard links";
    CHANGED = "why-changed", "the file changed on disk since it was compared; compare again";
    NO_SUMS = "why-no-sums", "no checksum lines in the file";
    NO_FILES = "why-no-files", "no files to sum (dirs are left out with a file for each file)";
    NOT_IN_TRASH = "why-not-in-trash", "not an item of the trash, or its trash info is missing";
}
