//! Settings (edited by hand, applied live) and state (written by the app): both cosmic-config.

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use shagoff_core::session::PaneState;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastTab {
    /// Ctrl+W on the last tab does nothing.
    #[default]
    Nothing,
    /// Ctrl+W on the last tab goes to `home_dir` (or `~`).
    Home,
}

/// `~/.config/cosmic/<APP_ID>/v1/<field>`.
#[derive(Clone, Debug, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct Config {
    pub show_hidden: bool,
    /// F3 program + args; empty → `xdg-open`.
    pub viewer: Vec<String>,
    /// F4 program + args; empty → `cosmic-edit`.
    pub editor: Vec<String>,
    pub last_tab_close: LastTab,
    pub home_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: false,
            viewer: Vec::new(),
            editor: vec!["cosmic-edit".into()],
            last_tab_close: LastTab::Nothing,
            home_dir: None,
        }
    }
}

/// `~/.local/state/cosmic/<APP_ID>/v1/<field>`.
#[derive(Clone, Debug, Default, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct State {
    pub panes: [PaneState; 2],
    pub active: usize,
}

pub fn config_handler() -> Option<cosmic_config::Config> {
    cosmic_config::Config::new(crate::app::APP_ID, Config::VERSION)
        .inspect_err(|e| log::warn!("config: {e}"))
        .ok()
}

pub fn state_handler() -> Option<cosmic_config::Config> {
    cosmic_config::Config::new_state(crate::app::APP_ID, State::VERSION)
        .inspect_err(|e| log::warn!("state: {e}"))
        .ok()
}

/// Stored value, or the default for anything missing or unreadable.
pub fn read<T: CosmicConfigEntry + Default>(h: Option<&cosmic_config::Config>) -> T {
    h.map(|h| T::get_entry(h).unwrap_or_else(|(_, c)| c))
        .unwrap_or_default()
}
