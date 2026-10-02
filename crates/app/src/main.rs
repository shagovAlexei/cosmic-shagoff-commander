mod app;
mod i18n;
mod keymap;

fn main() -> cosmic::iced::Result {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .ok();
    i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
    cosmic::app::run::<app::App>(cosmic::app::Settings::default(), ())
}
