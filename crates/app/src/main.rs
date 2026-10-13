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
    pick_renderer();
    i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
    let flags = app::Flags {
        left: std::env::args_os().nth(1).map(Into::into),
    };
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(1200.0, 800.0))
        // The window stays on × / Alt+F4: `Message::Quit` asks first while a job runs.
        .exit_on_close(false);
    cosmic::app::run::<app::App>(settings, flags)
}

/// iced's wgpu shaders need `SHADER_F16_IN_F32`. An old or software OpenGL (X11 without DRI3, as
/// on Linux Lite in a VM) lacks it and the app panicked on its first frame. There: the software
/// renderer. `ICED_BACKEND` set by hand wins. Costs ~70 ms at startup.
fn pick_renderer() {
    if std::env::var_os("ICED_BACKEND").is_some() {
        return;
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all()),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }));
    let f16 = wgpu::DownlevelFlags::SHADER_F16_IN_F32;
    if !adapter.is_ok_and(|a| a.get_downlevel_capabilities().flags.contains(f16)) {
        log::warn!("GPU without SHADER_F16_IN_F32: software rendering (ICED_BACKEND=tiny-skia)");
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    }
}
