//! UI-free core of Shagoff Commander: panel state, directory listing, file operations.
//! Must not depend on libcosmic so everything here stays unit-testable.

pub mod drives;
pub mod format;
pub mod history;
pub mod launch;
pub mod listing;
pub mod mask;
pub mod ops;
pub mod panel;
pub mod quicksearch;
pub mod session;
pub mod sort;
pub mod tabs;
pub mod viewport;
