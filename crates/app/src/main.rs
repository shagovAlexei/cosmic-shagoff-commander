mod app;
mod clip;
mod config;
mod dialogs;
mod drawer;
mod find;
mod help;
mod hotlist;
mod i18n;
mod jobs;
mod keymap;
mod lister;
mod menu;
mod toolbar;
mod view;
mod watcher;

fn main() -> cosmic::iced::Result {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .ok();
    i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
    let flags = app::Flags {
        left: std::env::args_os().nth(1).map(Into::into),
    };
    let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(1200.0, 800.0));
    cosmic::app::run::<app::App>(settings, flags)
}
