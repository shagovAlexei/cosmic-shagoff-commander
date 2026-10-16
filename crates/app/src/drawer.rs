//! Side drawer: help (F1), about, settings (Ctrl+,).

use crate::app::{APP_ID, App, Message};
use crate::config::{AppTheme, LANGUAGES, LastTab, Skin};
use crate::fl;
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, about::About, settings};
use cosmic::{Element, theme};
use shagoff_core::archive::Format;
use shagoff_core::update::Status;

pub const REPO: &str = "https://github.com/shagovAlexei/cosmic-shagoff-commander";

/// Where "Support the project" leads: a Stripe Payment Link, which needs no server.
pub const DONATE_URL: &str = "https://donate.stripe.com/6oU28s8Ff7cR1342J967S00";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drawer {
    Help,
    About,
    Donate,
    Settings(SettingsForm),
}

/// Text fields as typed: the config keeps parsed values (a command split on spaces would eat
/// the space being typed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsForm {
    pub viewer: String,
    pub editor: String,
    pub home: String,
    /// Installed font families, read when the settings open (not on every frame).
    pub fonts: Vec<String>,
}

/// The installed font families, sorted, each once.
pub fn font_families() -> Vec<String> {
    let mut fs = cosmic::iced::advanced::graphics::text::font_system()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<String> = fs
        .raw()
        .db()
        .faces()
        .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[derive(Clone, Debug)]
pub enum Setting {
    /// Index into [system, LANGUAGES…].
    Language(usize),
    Theme(usize),
    /// Index into `Skin::ALL`.
    Skin(usize),
    ShowFkeys(bool),
    DividerLine(bool),
    HumanSizes(bool),
    ActiveFrame(bool),
    ShowCmdline(bool),
    KeepHistory(bool),
    InternalViewer(bool),
    ShowHidden(bool),
    /// Ctrl+W on the last tab goes home.
    LastTabHome(bool),
    RightMarks(bool),
    ListerWindow(bool),
    /// F12: 0 — own program, else `toolbar::builtins()[i - 1]`.
    F12(usize),
    F12Cmd(String),
    F12Params(String),
    F12Tip(String),
    HomeDir(String),
    Viewer(String),
    Editor(String),
    PackFormat(usize),
    /// Index into [the interface's, `SettingsForm::fonts`…].
    PanelFont(usize),
    PanelSize(u16),
    ListerSize(u16),
}

pub fn about() -> [About; 2] {
    let head = About::default()
        .name(fl!("app-title"))
        .icon(widget::icon::from_name(APP_ID))
        .version(env!("CARGO_PKG_VERSION"))
        .author("Shagov Alexei");
    let rest = About::default()
        .comments(fl!("about-comments"))
        .license("GPL-3.0-only")
        .license_url("https://www.gnu.org/licenses/gpl-3.0.html")
        .links([
            (fl!("about-source"), REPO.to_string()),
            (fl!("about-issues"), format!("{REPO}/issues")),
        ]);
    [head, rest]
}

pub fn view<'a>(app: &'a App, d: &'a Drawer) -> ContextDrawer<'a, Message> {
    match d {
        Drawer::Help => {
            context_drawer::context_drawer(help(), Message::CloseDrawer).title(fl!("menu-help"))
        }
        Drawer::About => {
            let content = widget::column::with_children(vec![
                donate(),
                widget::about(&app.about[0], |url| Message::OpenUrl(url.to_string())),
                update_check(app.update_check.as_ref()),
                widget::about(&app.about[1], |url| Message::OpenUrl(url.to_string())),
            ])
            .spacing(24);
            context_drawer::context_drawer(content, Message::CloseDrawer).title(fl!("menu-about"))
        }
        Drawer::Donate => context_drawer::context_drawer(donate(), Message::CloseDrawer)
            .title(fl!("donate-title")),
        Drawer::Settings(form) => {
            context_drawer::context_drawer(settings_view(app, form), Message::CloseDrawer)
                .title(fl!("settings-title"))
        }
    }
}

/// The keys of a help row: each alternative ("F8, Delete", "Ctrl+C / Ctrl+X") its own label,
/// stacked, so long rows fit the drawer.
fn keys_view(keys: &'static str) -> Element<'static, Message> {
    let mut col = widget::column::with_capacity(3)
        .spacing(4)
        .align_x(Alignment::End);
    for k in keys.split(", ").flat_map(|k| k.split(" / ")) {
        col = col.push(key_label(k));
    }
    col.into()
}

/// A key as a small rounded label.
fn key_label(keys: &'static str) -> Element<'static, Message> {
    widget::container(
        widget::text::body(keys)
            .font(cosmic::font::mono())
            .wrapping(cosmic::iced::widget::text::Wrapping::None),
    )
    .padding([2, 8])
    .class(theme::Container::custom(|t| {
        let c = t.cosmic();
        widget::container::Style {
            background: Some(cosmic::iced::Color::from(c.background(false).component.base).into()),
            border: cosmic::iced::Border {
                radius: c.corner_radii.radius_s.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }))
    .into()
}

fn help() -> Element<'static, Message> {
    let mut col = widget::column::with_capacity(16).spacing(16).push(
        widget::column::with_children(vec![
            widget::text::title3(fl!("app-title")).into(),
            widget::text::body(fl!("help-intro")).into(),
            widget::text::body(fl!("help-rules")).into(),
        ])
        .spacing(8),
    );
    for (title, rows) in crate::help::sections() {
        let mut s = settings::section().title(title);
        for (keys, what, _) in rows {
            // The text takes what the keys leave: a long one no longer pushes them off the edge.
            s = s.add(settings::item_row(vec![
                widget::text::body(what).width(Length::Fill).into(),
                keys_view(keys),
            ]));
        }
        col = col.push(s);
    }
    col.into()
}

/// «Check for updates» and what it found: up to date, a newer release (with its link), or why not.
fn update_check(state: Option<&Status>) -> Element<'static, Message> {
    let mut button = widget::button::standard(fl!("update-check"));
    if state != Some(&Status::Checking) {
        button = button.on_press(Message::CheckUpdate);
    }
    let mut col = widget::column::with_capacity(3)
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .push(button);
    let text = |s: String| widget::text::body(s);
    match state {
        None => {}
        Some(Status::Checking) => col = col.push(text(fl!("update-checking"))),
        Some(Status::Latest) => col = col.push(text(fl!("update-latest"))),
        Some(Status::Available(tag)) => {
            col = col
                .push(text(fl!("update-available", tag = tag.as_str())))
                .push(
                    widget::button::suggested(fl!("update-open"))
                        .on_press(Message::OpenUrl(format!("{REPO}/releases/tag/{tag}"))),
                );
        }
        Some(Status::Failed(e)) => col = col.push(text(fl!("update-failed", error = e.as_str()))),
    }
    col.into()
}

fn donate() -> Element<'static, Message> {
    let button = widget::button::suggested(fl!("donate-button"))
        .on_press(Message::OpenUrl(DONATE_URL.to_string()));
    widget::column::with_capacity(3)
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .push(widget::text::heading(fl!("donate-title")))
        .push(widget::text::body(fl!("donate-text")))
        .push(button)
        .into()
}

/// A text size from `config::SIZES`.
/// The default one is labelled so.
fn size_pick<'a>(now: u16, default: u16, f: fn(u16) -> Setting) -> Element<'a, Message> {
    let sizes: Vec<String> = crate::config::SIZES
        .map(|s| match s == default {
            true => fl!("settings-size-default", size = s),
            false => s.to_string(),
        })
        .collect();
    let at = crate::config::SIZES.clone().position(|s| s == now);
    widget::dropdown(sizes, at, move |i| {
        Message::Setting(f(*crate::config::SIZES.start() + i as u16))
    })
    .into()
}

fn settings_view<'a>(app: &'a App, form: &'a SettingsForm) -> Element<'a, Message> {
    let c = &app.config;
    let set = |s: Setting| Message::Setting(s);
    let language = LANGUAGES
        .iter()
        .position(|l| *l == c.language)
        .map_or(0, |i| i + 1);
    let languages = vec![fl!("settings-system"), "English".into(), "Русский".into()];
    let themes = vec![
        fl!("settings-system"),
        fl!("settings-light"),
        fl!("settings-dark"),
    ];
    let theme = AppTheme::ALL.iter().position(|t| *t == c.app_theme);
    let skins = vec![fl!("settings-skin-classic"), fl!("settings-skin-modern")];
    let skin = Skin::ALL.iter().position(|s| *s == c.skin);
    let formats: Vec<&'static str> = Format::PACK.iter().map(|f| f.ext()).collect();
    let format = Format::PACK.iter().position(|f| f.ext() == c.pack_format);
    let input = |value: &'a str, placeholder: &'static str, f: fn(String) -> Setting| {
        widget::text_input(placeholder, value)
            .on_input(move |s| Message::Setting(f(s)))
            // Without on_tab, Tab leaves the field focused but read-only and no longer capturing
            // keys: Backspace / Delete would reach the panels (go up, delete).
            .on_tab(Message::FocusNext)
            .width(Length::Fixed(200.0))
    };
    widget::column::with_children(vec![
        settings::section()
            .title(fl!("settings-ui"))
            .add(settings::item(
                fl!("settings-language"),
                widget::dropdown(languages, Some(language), |i| {
                    Message::Setting(Setting::Language(i))
                }),
            ))
            .add(settings::item(
                fl!("settings-theme"),
                widget::dropdown(themes, theme, |i| Message::Setting(Setting::Theme(i))),
            ))
            .add(settings::item(
                fl!("settings-skin"),
                widget::dropdown(skins, skin, |i| Message::Setting(Setting::Skin(i))),
            ))
            .add(settings::item(
                fl!("settings-divider-line"),
                widget::toggler(c.divider_line).on_toggle(move |b| set(Setting::DividerLine(b))),
            ))
            .add(settings::item(
                fl!("settings-human-sizes"),
                widget::toggler(c.human_sizes()).on_toggle(move |b| set(Setting::HumanSizes(b))),
            ))
            .add_maybe((c.skin == Skin::Classic).then(|| {
                settings::item(
                    fl!("settings-active-frame"),
                    widget::toggler(c.active_frame)
                        .on_toggle(move |b| set(Setting::ActiveFrame(b))),
                )
            }))
            .add(settings::item(
                fl!("settings-fkeys"),
                widget::toggler(c.show_fkeys).on_toggle(move |b| set(Setting::ShowFkeys(b))),
            ))
            .add(settings::item(
                fl!("settings-cmdline"),
                widget::toggler(c.show_cmdline).on_toggle(move |b| set(Setting::ShowCmdline(b))),
            ))
            .add(settings::item(
                fl!("settings-keep-history"),
                widget::toggler(c.keep_history).on_toggle(move |b| set(Setting::KeepHistory(b))),
            ))
            .into(),
        settings::section()
            .title(fl!("settings-panels"))
            .add(settings::item(
                fl!("settings-hidden"),
                widget::toggler(c.show_hidden).on_toggle(move |b| set(Setting::ShowHidden(b))),
            ))
            .add(settings::item(
                fl!("settings-last-tab"),
                widget::toggler(c.last_tab_close == LastTab::Home)
                    .on_toggle(move |b| set(Setting::LastTabHome(b))),
            ))
            .add(settings::item(
                fl!("settings-right-marks"),
                widget::toggler(c.right_marks).on_toggle(move |b| set(Setting::RightMarks(b))),
            ))
            .add(settings::item(
                fl!("settings-home-dir"),
                input(&form.home, "~", Setting::HomeDir),
            ))
            .into(),
        f12_section(c),
        settings::section()
            .title(fl!("settings-programs"))
            .add(settings::item(
                fl!("settings-internal-viewer"),
                widget::toggler(c.internal_viewer)
                    .on_toggle(move |b| set(Setting::InternalViewer(b))),
            ))
            .add(settings::item(
                fl!("settings-lister-window"),
                widget::toggler(c.lister_window).on_toggle(move |b| set(Setting::ListerWindow(b))),
            ))
            .add(settings::item(
                fl!("settings-viewer"),
                input(&form.viewer, "xdg-open", Setting::Viewer),
            ))
            .add(settings::item(
                fl!("settings-editor"),
                input(&form.editor, "cosmic-edit", Setting::Editor),
            ))
            .into(),
        settings::section()
            .title(fl!("settings-fonts"))
            .add(settings::item(fl!("settings-panel-font"), {
                let names: Vec<String> = std::iter::once(fl!("settings-system"))
                    .chain(form.fonts.iter().cloned())
                    .collect();
                let at = form
                    .fonts
                    .iter()
                    .position(|f| *f == c.panel_font)
                    .map_or(0, |i| i + 1);
                widget::dropdown(names, Some(at), |i| Message::Setting(Setting::PanelFont(i)))
            }))
            .add(settings::item(
                fl!("settings-panel-size"),
                size_pick(c.panel_size, crate::config::PANEL_SIZE, Setting::PanelSize),
            ))
            .add(settings::item(
                fl!("settings-lister-size"),
                size_pick(
                    c.lister_size,
                    crate::config::LISTER_SIZE,
                    Setting::ListerSize,
                ),
            ))
            .into(),
        settings::section()
            .title(fl!("settings-archives"))
            .add(settings::item(
                fl!("settings-pack-format"),
                widget::dropdown(formats, format, |i| {
                    Message::Setting(Setting::PackFormat(i))
                }),
            ))
            .into(),
    ])
    .spacing(16)
    .into()
}

/// F12: an internal command from the list, or an own program with parameters.
fn f12_section(c: &crate::config::Config) -> Element<'_, Message> {
    let set = |s| Message::Setting(s);
    let cmds: Vec<String> = std::iter::once(fl!("settings-f12-own"))
        .chain(crate::toolbar::builtins().iter().map(|b| (b.label)()))
        .collect();
    let at = (crate::toolbar::builtins().iter())
        .position(|b| b.cm == c.f12.cmd.trim())
        .map_or(0, |i| i + 1);
    let mut section = settings::section()
        .title(fl!("settings-f12"))
        .add(settings::item(
            fl!("settings-f12-command"),
            widget::dropdown(cmds, Some(at), move |i| set(Setting::F12(i))),
        ));
    if at == 0 {
        let field = |value: &'static str, v: &str, f: fn(String) -> Setting| {
            widget::text_input(value, v.to_string())
                .on_input(move |s| set(f(s)))
                .on_tab(Message::FocusNext)
                .width(Length::Fixed(200.0))
        };
        section = section
            .add(settings::item(
                fl!("settings-f12-program"),
                field("cosmic-term", &c.f12.cmd, Setting::F12Cmd),
            ))
            .add(settings::item(
                fl!("settings-f12-params"),
                field("%P%N", &c.f12.params, Setting::F12Params),
            ))
            .add(settings::item(
                fl!("settings-f12-tip"),
                field("", &c.f12.tip, Setting::F12Tip),
            ));
    }
    section.into()
}
