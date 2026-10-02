//! UI-free core of Shagoff Commander: panel state, directory listing, file operations.
//! Must not depend on libcosmic so everything here stays unit-testable.

pub mod format;
pub mod listing;
pub mod mask;
pub mod ops;
pub mod panel;
pub mod sort;
pub mod tabs;
pub mod viewport;
