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
            // The user's COSMIC palette (accent…), pinned to light / dark. `Custom`, not
            // `System` + prefer_dark: libcosmic replaces a System theme with the system's
            // own palette on its startup theme event, so "Light" on a dark desktop was lost.
            AppTheme::Light => pinned(cosmic::theme::system_light(), false),
            AppTheme::Dark => pinned(cosmic::theme::system_dark(), true),
        }
    }
}

fn pinned(t: cosmic::Theme, dark: bool) -> cosmic::Theme {
    use cosmic::cosmic_theme::Theme as Palette;
    let mut p = t.cosmic().clone();
    // Outside COSMIC (Xfce on Linux Lite: no theme config, no /usr/share/cosmic) libcosmic hands
    // back its default palette, dark: "Light" stayed dark. Then COSMIC's stock one.
    if p.is_dark != dark {
        p = if dark {
            Palette::dark_default()
        } else {
            Palette::light_default()
        };
    }
    cosmic::Theme::custom(std::sync::Arc::new(p))
}

/// Look of the window (Ctrl+, → Interface): the same layout and keys, drawn differently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Skin {
    /// Dense, like Total Commander: column rules, toolbar, drives in a list.
    #[default]
    Classic,
    /// COSMIC style: cards, breadcrumbs, roomier rows.
    Modern,
}

impl Skin {
    pub const ALL: [Skin; 2] = [Skin::Classic, Skin::Modern];

    /// Height of a file row; the scroll math is built on it.
    pub fn row_h(self) -> f32 {
        match self {
            Skin::Classic => 20.0,
            Skin::Modern => 28.0,
        }
    }
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
    /// Files → «Create checksum files»: the last algorithm picked (`Algo::ext`).
    pub checksum_algo: String,
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
    /// View → Window parts (TC `cm_Vis*`); the F-keys and the command line are the two above.
    pub show_toolbar: bool,
    pub show_drives: bool,
    pub show_tabs: bool,
    pub show_path: bool,
    pub show_header: bool,
    pub show_status: bool,
    /// Command line, "Other program…" and mask histories survive a restart (off: kept in memory only).
    pub keep_history: bool,
    /// At start (once a day): ask GitHub for a newer release and offer it.
    pub check_updates: bool,
    /// Shift+Enter in the command line: terminal program + args; the command (`sh -c …`) is appended.
    pub terminal: Vec<String>,
    /// Commands → Terminal: the `.desktop` id of the terminal chosen; empty: the system's default.
    /// Shift+Enter uses it unless `terminal` was changed from its default by hand.
    pub terminal_app: String,
    /// View → Built-in terminal: under the panels in place of the command line.
    pub term_builtin: bool,
    /// Commands → Shell…: its path (`/etc/shells`); empty: `$SHELL`.
    pub term_shell: String,
    /// Its height under the panels, in rows (its top edge dragged).
    pub term_rows: u16,
    pub skin: Skin,
    /// TC "Select with right mouse button": a right click marks, a long press opens the menu;
    /// off: a right click opens the menu at once.
    pub right_marks: bool,
    /// F3 in a window of its own (TC Lister); off: in place of the panels.
    pub lister_window: bool,
    /// A thin line in the gap between the panes.
    pub divider_line: bool,
    /// File sizes in KB / MB / GB (`true`) or in bytes; `None`: as the skin has them (Modern in
    /// KB / MB, Classic in bytes up to 999 999 999).
    pub human_sizes: Option<bool>,
    /// The classic look: the active pane framed in the accent colour, as the modern look's card.
    pub active_frame: bool,
    /// F12: an internal command (`cm_*`) or a program with parameters, as a toolbar button;
    /// none set: F12 opens the settings.
    pub f12: ToolButton,
    /// Full view widths of Ext, Size, Date, Attributes (Name takes the rest); dragged in the header.
    pub columns: [f32; 4],
    /// Which columns the full view shows after Name (`shagoff_core::columns`, by key).
    pub col_set: Vec<String>,
    /// View → Columns: sets of columns to pick from, saved by name.
    pub view_modes: Vec<ViewMode>,
    /// The classic skin's button bar (TC "Change button bar"); default: `toolbar::default_bar`.
    pub toolbar: Vec<ToolButton>,
    /// TC's Start menu: its items as buttons (`tip` the item's name; a separator is `toolbar::SEP`).
    pub start_menu: Vec<ToolButton>,
    /// The user's own keys (Configuration → Hotkeys): ("Ctrl+K", action name — `hotkeys::name`
    /// — or "" to turn the key off), tried before the default table.
    pub hotkeys: Vec<(String, String)>,
    /// Name colours by type (TC "Color by file type"), first match wins.
    pub colors: Vec<shagoff_core::colors::Rule>,
    /// The file list's colours: its text and background, marked rows, the cursor; empty: the theme's.
    pub panel: shagoff_core::colors::PanelColors,
    /// The file list's font family; "" = the interface's.
    pub panel_font: String,
    /// The file list's text size, pt (13: the skins' own).
    pub panel_size: u16,
    /// F3 viewer's text size (its font stays monospaced: hex and wrap count its columns).
    pub lister_size: u16,
    /// TC ignore list (`cm_SwitchIgnoreList`): `ignore_list` names / masks (`;`, as Ctrl+F12) hidden.
    pub ignore_on: bool,
    pub ignore_list: String,
    /// Dirs watched for changes (TC `cm_SwitchWatchDirs`); off: Ctrl+R reads them again.
    pub watch_dirs: bool,
    /// Dirs always by name (TC `cm_SwitchDirSort`).
    pub dirs_by_name: bool,
    /// The colors by file type in force (TC `cm_SwitchColorsByFileType`); off: plain rows.
    pub type_colors: bool,
    /// Thumbnails view: the picture's side, px (View menu, TC `cm_ZoomIn` / `cm_ZoomOut`; no key).
    pub thumb_size: u32,
    /// Thumbnails are kept in the shared disk cache (`~/.cache/thumbnails`); off: made each time.
    pub thumb_cache: bool,
    /// Copies are read back from the disk and compared with the source (slower).
    pub verify_copy: bool,
}

/// Text sizes offered in the settings.
pub const SIZES: std::ops::RangeInclusive<u16> = 9..=24;

/// One size up or down, kept in `SIZES` (the viewer's + / -).
pub fn size_step(now: u16, up: bool) -> u16 {
    let next = if up {
        now.saturating_add(1)
    } else {
        now.saturating_sub(1)
    };
    next.clamp(*SIZES.start(), *SIZES.end())
}
pub const PANEL_SIZE: u16 = 13;
pub const LISTER_SIZE: u16 = 14;

impl Config {
    /// The row colors: by file type unless switched off, the panel's own either way.
    pub fn scheme(&self) -> shagoff_core::colors::Scheme {
        let rules: &[shagoff_core::colors::Rule] = match self.type_colors {
            true => &self.colors,
            false => &[],
        };
        shagoff_core::colors::Scheme::new(rules).with_panel(&self.panel)
    }

    /// The program-wide switches core keeps (`sort::set_dirs_by_name`, `panel::set_ignore`).
    pub fn set_core_switches(&self) {
        crate::thumbs::set_size(self.thumb_size);
        shagoff_core::ops::set_verify(self.verify_copy);
        shagoff_core::sort::set_dirs_by_name(self.dirs_by_name);
        let list = self.ignore_list.trim();
        // Only exclusions (`|*.tmp`), or `;` alone: `Mask` reads that as `*`, i.e. all hidden.
        let names = list.split('|').next().unwrap_or("");
        let names = names.trim_matches(|c: char| c == ';' || c == '"' || c.is_whitespace());
        let on = self.ignore_on && !names.is_empty();
        shagoff_core::panel::set_ignore(on.then(|| shagoff_core::mask::Mask::parse(list)));
    }

    /// Sizes in KB / MB / GB: as chosen, else as the skin has them.
    pub fn human_sizes(&self) -> bool {
        shagoff_core::format::human_size(0, self.human_sizes, self.skin == Skin::Modern)
    }

    /// The file list's text size, kept in `SIZES` (a hand edit could say anything).
    pub fn panel_size(&self) -> u16 {
        self.panel_size.clamp(*SIZES.start(), *SIZES.end())
    }

    pub fn lister_size(&self) -> u16 {
        self.lister_size.clamp(*SIZES.start(), *SIZES.end())
    }

    /// The file list's text size over the default.
    pub fn text_scale(&self) -> f32 {
        self.panel_size() as f32 / PANEL_SIZE as f32
    }

    /// A panel row's height: the skin's, grown or shrunk with the text.
    pub fn row_h(&self) -> f32 {
        (self.skin.row_h() * self.text_scale()).round()
    }
}

/// A toolbar button: `cmd` is an internal `cm_*` command, a program (`params` with `%P`, `%N`, …
/// appended, run by `sh -c` in `dir`, `cmdline::start_dir`), or `-` (a gap). Empty `icon` / `tip`:
/// the command's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolButton {
    pub cmd: String,
    pub params: String,
    pub icon: String,
    pub tip: String,
    pub dir: String,
}

/// A favourite dir (Ctrl+D).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotEntry {
    pub name: String,
    pub path: PathBuf,
}

/// A named set of columns of the full view (View → Columns → modes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewMode {
    pub name: String,
    /// `shagoff_core::columns::Col::key`s.
    pub cols: Vec<String>,
}

/// The modes a new install has: TC's default four, a short one, a long one, names only.
pub fn default_view_modes() -> Vec<ViewMode> {
    let m = |name: &str, cols: &[&str]| ViewMode {
        name: name.into(),
        cols: cols.iter().map(|c| c.to_string()).collect(),
    };
    vec![
        m(
            &crate::fl!("mode-standard"),
            &["ext", "size", "date", "attr"],
        ),
        m(&crate::fl!("mode-short"), &["size", "date"]),
        m(
            &crate::fl!("mode-detailed"),
            &["ext", "size", "date", "attr", "owner", "group"],
        ),
        m(&crate::fl!("mode-names-only"), &[]),
    ]
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
            checksum_algo: "sha256".into(),
            language: String::new(),
            app_theme: AppTheme::System,
            show_fkeys: true,
            internal_viewer: true,
            connections: Vec::new(),
            show_cmdline: true,
            show_toolbar: true,
            show_drives: true,
            show_tabs: true,
            show_path: true,
            show_header: true,
            show_status: true,
            keep_history: true,
            check_updates: true,
            terminal: vec!["cosmic-term".into(), "-e".into()],
            terminal_app: String::new(),
            term_builtin: false,
            term_shell: String::new(),
            term_rows: 8,
            skin: Skin::Classic,
            right_marks: true,
            lister_window: true,
            divider_line: false,
            human_sizes: None,
            active_frame: false,
            f12: ToolButton::default(),
            columns: [60.0, 90.0, 130.0, 80.0],
            col_set: shagoff_core::columns::default_keys(),
            view_modes: default_view_modes(),
            toolbar: crate::toolbar::default_bar(),
            start_menu: Vec::new(),
            hotkeys: Vec::new(),
            colors: shagoff_core::colors::defaults(),
            panel: shagoff_core::colors::default_panel(),
            panel_font: String::new(),
            panel_size: PANEL_SIZE,
            lister_size: LISTER_SIZE,
            ignore_on: false,
            ignore_list: String::new(),
            watch_dirs: true,
            dirs_by_name: false,
            type_colors: true,
            thumb_size: crate::thumbs::SIZE,
            thumb_cache: true,
            verify_copy: false,
        }
    }
}

/// A part of the window View → Window parts shows or hides (TC `cm_Vis*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Toolbar,
    Drives,
    Tabs,
    Path,
    Header,
    Status,
    Cmdline,
    Fkeys,
}

impl Part {
    pub const ALL: [Part; 8] = [
        Part::Toolbar,
        Part::Drives,
        Part::Tabs,
        Part::Path,
        Part::Header,
        Part::Status,
        Part::Cmdline,
        Part::Fkeys,
    ];

    pub fn label(self) -> String {
        match self {
            Part::Toolbar => crate::fl!("vis-toolbar"),
            Part::Drives => crate::fl!("vis-drives"),
            Part::Tabs => crate::fl!("vis-tabs"),
            Part::Path => crate::fl!("vis-path"),
            Part::Header => crate::fl!("vis-header"),
            Part::Status => crate::fl!("vis-status"),
            Part::Cmdline => crate::fl!("vis-cmdline"),
            Part::Fkeys => crate::fl!("vis-fkeys"),
        }
    }
}

impl Config {
    pub fn shows(&self, p: Part) -> bool {
        let c = self;
        match p {
            Part::Toolbar => c.show_toolbar,
            Part::Drives => c.show_drives,
            Part::Tabs => c.show_tabs,
            Part::Path => c.show_path,
            Part::Header => c.show_header,
            Part::Status => c.show_status,
            Part::Cmdline => c.show_cmdline,
            Part::Fkeys => c.show_fkeys,
        }
    }

    pub fn shown(&mut self, p: Part) -> &mut bool {
        match p {
            Part::Toolbar => &mut self.show_toolbar,
            Part::Drives => &mut self.show_drives,
            Part::Tabs => &mut self.show_tabs,
            Part::Path => &mut self.show_path,
            Part::Header => &mut self.show_header,
            Part::Status => &mut self.show_status,
            Part::Cmdline => &mut self.show_cmdline,
            Part::Fkeys => &mut self.show_fkeys,
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
    /// Ctrl+Shift+S: the last compare's options.
    pub sync: SyncPrefs,
    /// The left pane's share of the width (the divider); 0 = never moved: half.
    pub split: f32,
    /// View → «Panel at 100 %» (TC `cm_100Percent`): only the active panel, the whole width.
    pub wide: bool,
    /// View → «Panels one above the other» (TC `cm_VerticalPanels`).
    pub vertical: bool,
    /// Ctrl+Shift+F8: the tree beside the panels.
    pub side_tree: bool,
    /// The version «Skip» in the update dialog put aside.
    pub update_skip: String,
    /// Command line history, last first.
    pub commands: Vec<String>,
    /// Num+ / Num− masks, last first.
    pub masks: Vec<String>,
    /// "Open with…" → "Other program…" commands, last first.
    pub other_cmds: Vec<String>,
}

/// What the find dialog opens with (the dir always comes from the panel). Fields missing in an
/// older state file take their defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FindPrefs {
    pub mask: String,
    /// «Искать текст» (TC's «Find text»): the text field and its options shown and used.
    pub find_text: bool,
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub name_regex: bool,
    pub archives: bool,
    /// As typed: KB, KB, days.
    pub min_size: String,
    pub max_size: String,
    pub days: String,
    /// `h r s x`, as typed.
    pub attrs: String,
}

impl Default for FindPrefs {
    fn default() -> Self {
        Self {
            mask: "*".into(),
            find_text: false,
            text: String::new(),
            case_sensitive: false,
            regex: false,
            name_regex: false,
            archives: false,
            min_size: String::new(),
            max_size: String::new(),
            days: String::new(),
            attrs: String::new(),
        }
    }
}

/// What the sync dialog opens with (the dirs come from the panels, hidden files from the panel's
/// view). Fields missing in an older state file take their defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncPrefs {
    pub recursive: bool,
    pub content: bool,
    pub ignore_date: bool,
    // Not «Зеркало»: on, it plans the right's extra files for the trash; it is turned on each time.
    pub mask: String,
    /// Kinds of rows hidden from the list.
    pub hide: Vec<shagoff_core::sync::Kind>,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        Self {
            recursive: true,
            content: false,
            ignore_date: false,
            mask: String::new(),
            // As before: equal files hidden until asked for.
            hide: vec![shagoff_core::sync::Kind::Same],
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
    // A bad value (say `"Home"` in quotes for an enum) falls back to its default: say which.
    h.map(|h| {
        T::get_entry(h).unwrap_or_else(|(errs, c)| {
            for e in errs.iter().filter(|e| e.is_err()) {
                log::warn!("config: {e}");
            }
            c
        })
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_steps_stay_in_bounds() {
        assert_eq!(size_step(14, true), 15);
        assert_eq!(size_step(14, false), 13);
        assert_eq!(size_step(*SIZES.end(), true), *SIZES.end());
        assert_eq!(size_step(*SIZES.start(), false), *SIZES.start());
    }

    #[test]
    fn rows_grow_with_the_panel_text() {
        let mut c = Config::default();
        assert_eq!(c.row_h(), Skin::Classic.row_h()); // the default size: as before
        c.panel_size = 24;
        assert_eq!(c.row_h(), (20.0 * 24.0 / 13.0_f32).round());
        c.skin = Skin::Modern;
        c.panel_size = 0; // a hand edit: kept in bounds
        assert_eq!(c.row_h(), (28.0 * 9.0 / 13.0_f32).round());
        c.panel_size = 5000;
        assert_eq!(c.panel_size(), 24);
    }

    #[test]
    fn regression_light_or_dark_is_not_a_system_theme() {
        // A System theme is swapped for the desktop's palette at startup (libcosmic).
        for t in [AppTheme::Light, AppTheme::Dark] {
            let ty = t.theme().theme_type;
            assert!(matches!(ty, cosmic::theme::ThemeType::Custom(_)), "{t:?}");
        }
    }

    #[test]
    fn regression_light_is_light_without_a_cosmic_theme_config() {
        // No theme config: libcosmic's default palette is dark.
        let dark = cosmic::theme::Theme::dark();
        assert!(!pinned(dark.clone(), false).cosmic().is_dark);
        assert!(pinned(dark, true).cosmic().is_dark);
    }

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
        // An old install: one field saved, one with a value this version cannot read, the rest
        // (fields added since) missing.
        let tmp = tempfile::tempdir().unwrap();
        let h = cosmic_config::Config::with_custom_path(
            crate::app::APP_ID,
            Config::VERSION,
            tmp.path().into(),
        )
        .unwrap();
        let dir = tmp.path().join(format!(
            "cosmic/{}/v{}",
            crate::app::APP_ID,
            Config::VERSION
        ));
        std::fs::write(dir.join("show_hidden"), "true").unwrap();
        std::fs::write(dir.join("app_theme"), "\"Bogus\"").unwrap();
        let c: Config = read(Some(&h));
        assert!(c.show_hidden); // kept
        assert_eq!(
            (c.language.as_str(), c.app_theme, c.show_fkeys),
            ("", AppTheme::System, true) // defaults
        );
    }
}
