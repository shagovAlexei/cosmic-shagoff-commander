mod app;
mod catch;
mod clip;
mod colorrules;
mod config;
mod dialogs;
mod dnd;
mod drawer;
mod find;
mod help;
mod hotkeys;
mod hotlist;
mod i18n;
mod jobs;
mod keymap;
mod lister;
mod menu;
mod term;
mod thumbs;
mod toolbar;
mod view;
mod watcher;
mod why;

fn main() -> cosmic::iced::Result {
    // Run as root by pkexec for Alt+Enter's owner / group / bits (163): no window.
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|a| a == shagoff_core::props::HELPER)
    {
        let code = match shagoff_core::props::run_helper(&args) {
            Ok(failed) => {
                failed.iter().for_each(|l| eprintln!("{l}"));
                i32::from(!failed.is_empty())
            }
            Err(e) => {
                eprintln!("{e}");
                2
            }
        };
        std::process::exit(code);
    }
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init()
        .ok();
    // A crash is kept for the next start to offer as an issue (214c).
    let crash = shagoff_core::crash::log_path();
    let last_crash = crash.as_deref().and_then(shagoff_core::crash::take);
    if let Some(log) = crash {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            let version = env!("CARGO_PKG_VERSION");
            shagoff_core::crash::write(&log, &format!("{info}\n\nthread: {thread}, v{version}\n"));
            default(info);
        }));
    }
    let software = pick_renderer();
    i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
    let flags = app::Flags {
        left: std::env::args_os().nth(1).map(Into::into),
        last_crash,
    };
    let mut settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(1200.0, 800.0))
        // The window stays on × / Alt+F4: `Message::Quit` asks first while a job runs.
        .exit_on_close(false)
        // softbuffer (tiny-skia) takes only 24-bit X11 visuals; a transparent window gets a 32-bit one.
        .transparent(!software);
    if let Some(theme) = desktop_icon_theme() {
        settings = settings.default_icon_theme(theme);
    }
    cosmic::app::run::<app::App>(settings, flags)
}

/// iced's wgpu shaders need `SHADER_F16_IN_F32`. An old or software OpenGL (X11 without DRI3, as
/// on Linux Lite in a VM) lacks it and the app panicked on its first frame. There: the software
/// renderer. `ICED_BACKEND` set by hand wins. Costs ~70 ms at startup. `true`: software.
fn pick_renderer() -> bool {
    if let Some(b) = std::env::var_os("ICED_BACKEND") {
        return b.to_string_lossy().starts_with("tiny-skia");
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
    let fits = |a: &wgpu::Adapter| {
        a.get_downlevel_capabilities().flags.contains(f16) && iced_device_fits(&a.limits())
    };
    if !adapter.is_ok_and(|a| fits(&a)) {
        log::warn!(
            "GPU short of what iced_wgpu needs: software rendering (ICED_BACKEND=tiny-skia)"
        );
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
        return true;
    }
    false
}

/// The weakest device iced_wgpu's compositor asks for (`downlevel_defaults` with its own bind
/// limits). Short of it (OpenGL 3.3 on an old Radeon), iced falls back to tiny-skia by itself,
/// after the window is made transparent, and softbuffer panics on its 32-bit X11 visual.
fn iced_device_fits(allowed: &wgpu::Limits) -> bool {
    let wanted = wgpu::Limits {
        max_bind_groups: 2,
        max_non_sampler_bindings: 2048,
        ..wgpu::Limits::downlevel_defaults()
    };
    wanted.check_limits_with_fail_fn(allowed, false, |name, want, have| {
        log::warn!("GPU limit {name}: {have} < {want}: software rendering");
    });
    wanted.check_limits(allowed)
}

/// libcosmic looks icons up in the "Cosmic" theme. Outside COSMIC (Xfce on Linux Lite) there is
/// none and the title bar's minimize / maximize came out blank: there, the desktop's own theme.
fn desktop_icon_theme() -> Option<String> {
    use shagoff_core::icons;
    let dirs = shagoff_core::openwith::data_dirs();
    if icons::theme_installed(cosmic::icon_theme::COSMIC, &dirs) {
        return None;
    }
    let run = |cmd: &str, args: &[&str]| {
        let out = shagoff_core::host::command(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| icons::theme_name(&String::from_utf8_lossy(&out.stdout)))?
    };
    let ini = || {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::home_dir().map(|h| h.join(".config")))?;
        icons::ini_theme(&std::fs::read_to_string(cfg.join("gtk-3.0/settings.ini")).ok()?)
    };
    run(
        "xfconf-query",
        &["-c", "xsettings", "-p", "/Net/IconThemeName"],
    )
    .or_else(ini)
    .or_else(|| {
        run(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "icon-theme"],
        )
    })
    .filter(|t| icons::theme_installed(t, &dirs))
}

#[cfg(test)]
mod tests {
    use super::iced_device_fits;

    #[test]
    fn regression_old_gl_gets_software_renderer() {
        // GL ES 3 / old desktop GL: no storage buffers, iced_wgpu cannot get its device.
        assert!(!iced_device_fits(&wgpu::Limits::downlevel_webgl2_defaults()));
        assert!(iced_device_fits(&wgpu::Limits::downlevel_defaults()));
        assert!(iced_device_fits(&wgpu::Limits::default()));
    }
}
