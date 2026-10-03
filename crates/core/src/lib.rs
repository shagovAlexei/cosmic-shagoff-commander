//! UI-free core of Shagoff Commander: panel state, directory listing, file operations.
//! Must not depend on libcosmic so everything here stays unit-testable.

pub mod archive;
pub mod clipboard;
pub mod diff;
pub mod drives;
pub mod format;
pub mod history;
pub mod launch;
pub mod listing;
pub mod mask;
pub mod mount;
pub mod multirename;
pub mod ops;
pub mod panel;
pub mod quicksearch;
pub mod search;
pub mod session;
pub mod sort;
pub mod sync;
pub mod tabs;
pub mod viewport;
