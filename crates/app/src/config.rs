//! Settings (edited by hand, applied live) and state (written by the app): both cosmic-config.

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use i18n_embed::unic_langid::LanguageIdentifier;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppTheme {
    /// Follow the COSMIC appearance setting.
    #[default]
    System,
    Light,
    Dark,
}

impl AppTheme {
    pub const ALL: [AppTheme; 3] = [AppTheme::System, AppTheme::Light, AppTheme::Dark];

    pub fn theme(self) -> cosmic::Theme {
        match self {
            AppTheme::System => cosmic::theme::system_preference(),
            // The user's COSMIC palette (accent…), pinned to light / dark.
            AppTheme::Light => pinned(cosmic::theme::system_light(), false),
            AppTheme::Dark => pinned(cosmic::theme::system_dark(), true),
        }
    }
}

fn pinned(mut t: cosmic::Theme, dark: bool) -> cosmic::Theme {
    t.theme_type.prefer_dark(Some(dark));
    t
}

/// Interface languages we ship; `""` in the config = the system's.
pub const LANGUAGES: [&str; 2] = ["en", "ru"];

/// What to ask the localizer for: the configured language, or the system's list.
pub fn languages(setting: &str, system: Vec<LanguageIdentifier>) -> Vec<LanguageIdentifier> {
    match setting.parse() {
        Ok(l) if LANGUAGES.contains(&setting) => vec![l],
        _ => system,
    }
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
    /// Ctrl+D favourites, in menu order.
    pub hotlist: Vec<HotEntry>,
    /// Alt+F5: last chosen format, by extension ("zip", "tar.gz", …); unknown → zip.
    pub pack_format: String,
    /// Interface language: "" (system), "en", "ru".
    pub language: String,
    pub app_theme: AppTheme,
    /// F-key buttons at the bottom.
    pub show_fkeys: bool,
    /// F3 opens the built-in viewer; off: runs `viewer`.
    pub internal_viewer: bool,
    /// Ctrl+F: addresses connected to, last first, without passwords.
    pub connections: Vec<String>,
    /// Command line under the panels (TC `path>`).
    pub show_cmdline: bool,
    /// Shift+Enter in the command line: terminal program + args; the command (`sh -c …`) is appended.
    pub terminal: Vec<String>,
}

/// A favourite dir (Ctrl+D).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotEntry {
    pub name: String,
    pub path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: false,
            viewer: Vec::new(),
            editor: vec!["cosmic-edit".into()],
            last_tab_close: LastTab::Nothing,
            home_dir: None,
            hotlist: Vec::new(),
            pack_format: "zip".into(),
            language: String::new(),
            app_theme: AppTheme::System,
            show_fkeys: true,
            internal_viewer: true,
            connections: Vec::new(),
            show_cmdline: true,
            terminal: vec!["cosmic-term".into(), "-e".into()],
        }
    }
}

/// `~/.local/state/cosmic/<APP_ID>/v1/<field>`.
#[derive(Clone, Debug, Default, PartialEq, CosmicConfigEntry)]
#[version = 1]
pub struct State {
    pub panes: [PaneState; 2],
    pub active: usize,
    /// Alt+F7: last search settings.
    pub find: FindPrefs,
    /// Command line history, last first.
    pub commands: Vec<String>,
}

/// What the find dialog opens with (the dir always comes from the panel).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindPrefs {
    pub mask: String,
    pub text: String,
    pub case_sensitive: bool,
}

impl Default for FindPrefs {
    fn default() -> Self {
        Self {
            mask: "*".into(),
            text: String::new(),
            case_sensitive: false,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_setting() {
        let sys: Vec<LanguageIdentifier> = vec!["de".parse().unwrap()];
        assert_eq!(languages("", sys.clone()), sys);
        assert_eq!(
            languages("ru", sys.clone()),
            ["ru".parse::<LanguageIdentifier>().unwrap()]
        );
        assert_eq!(languages("xx", sys.clone()), sys); // not one we ship
    }

    #[test]
    fn old_config_gets_new_defaults() {
        let c = Config::default();
        assert_eq!(
            (c.language.as_str(), c.app_theme, c.show_fkeys),
            ("", AppTheme::System, true)
        );
    }
}
