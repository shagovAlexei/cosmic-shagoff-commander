//! Side drawer: help (F1), about, settings (Ctrl+,).

use crate::app::{APP_ID, App, Message};
use crate::config::{AppTheme, LANGUAGES, LastTab};
use crate::fl;
use cosmic::app::context_drawer::{self, ContextDrawer};
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, about::About, settings};
use cosmic::{Element, theme};
use shagoff_core::archive::Format;

pub const REPO: &str = "https://github.com/shagovAlexei/cosmic-shagoff-commander";

/// Where "Support the project" leads: a Stripe Payment Link (`https://buy.stripe.com/…`), which
/// needs no server. `None` until it exists: the button is shown disabled.
pub const DONATE_URL: Option<&str> = None;

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
}

#[derive(Clone, Debug)]
pub enum Setting {
    /// Index into [system, LANGUAGES…].
    Language(usize),
    Theme(usize),
    ShowFkeys(bool),
    ShowCmdline(bool),
    InternalViewer(bool),
    ShowHidden(bool),
    /// Ctrl+W on the last tab goes home.
    LastTabHome(bool),
    HomeDir(String),
    Viewer(String),
    Editor(String),
    PackFormat(usize),
}

pub fn about() -> About {
    About::default()
        .name(fl!("app-title"))
        .icon(widget::icon::from_name(APP_ID))
        .version(env!("CARGO_PKG_VERSION"))
        .author("Shagov Alexei")
        .comments(fl!("about-comments"))
        .license("GPL-3.0-only")
        .license_url("https://www.gnu.org/licenses/gpl-3.0.html")
        .links([
            (fl!("about-source"), REPO.to_string()),
            (fl!("about-issues"), format!("{REPO}/issues")),
        ])
}

pub fn view<'a>(app: &'a App, d: &'a Drawer) -> ContextDrawer<'a, Message> {
    match d {
        Drawer::Help => {
            context_drawer::context_drawer(help(), Message::CloseDrawer).title(fl!("menu-help"))
        }
        Drawer::About => {
            let content = widget::column::with_children(vec![
                donate(),
                widget::about(&app.about, |url| Message::OpenUrl(url.to_string())),
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
            s = s.add(settings::item(what, keys_view(keys)));
        }
        col = col.push(s);
    }
    col.into()
}

fn donate() -> Element<'static, Message> {
    let button = widget::button::suggested(fl!("donate-button"));
    let button = match DONATE_URL {
        Some(url) => button.on_press(Message::OpenUrl(url.to_string())),
        None => button,
    };
    let mut col = widget::column::with_capacity(4)
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .push(widget::text::heading(fl!("donate-title")))
        .push(widget::text::body(fl!("donate-text")))
        .push(button);
    if DONATE_URL.is_none() {
        col = col.push(widget::text::caption(fl!("donate-soon")));
    }
    col.into()
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
                fl!("settings-fkeys"),
                widget::toggler(c.show_fkeys).on_toggle(move |b| set(Setting::ShowFkeys(b))),
            ))
            .add(settings::item(
                fl!("settings-cmdline"),
                widget::toggler(c.show_cmdline).on_toggle(move |b| set(Setting::ShowCmdline(b))),
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
                fl!("settings-home-dir"),
                input(&form.home, "~", Setting::HomeDir),
            ))
            .into(),
        settings::section()
            .title(fl!("settings-programs"))
            .add(settings::item(
                fl!("settings-internal-viewer"),
                widget::toggler(c.internal_viewer)
                    .on_toggle(move |b| set(Setting::InternalViewer(b))),
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
