//! Built-in terminal (phase 149): cosmic-term's terminal under the panels, in place of the
//! command line. Its shell follows the active panel's dir while it waits at its prompt.

mod mouse_reporter;
pub mod style;
pub mod terminal;
pub mod terminal_box;
pub mod terminal_theme;

use crate::app::Message;
use alacritty_terminal::{event::Event, term, tty};
use cosmic::iced::{Subscription, futures::SinkExt, keyboard::Key, keyboard::Modifiers};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use terminal::Terminal;
use tokio::sync::mpsc::UnboundedSender;

/// Lines kept above the screen.
const HISTORY: usize = 10_000;

pub struct Pane {
    pub terminal: Mutex<Terminal>,
    /// The dir its shell was last sent to (or started in).
    dir: Option<PathBuf>,
    /// Its path: whether it has a line editor (`shell::at_prompt`).
    shell: String,
    /// When it was last found busy: asked again only after `RETRY` (not on every message of a
    /// long build's output).
    busy_at: Option<std::time::Instant>,
}

const RETRY: std::time::Duration = std::time::Duration::from_millis(300);
/// While a `cd` waits (`waiting_tick`): slow, a long program (vim, a build) may hold the shell.
const WAIT_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// Start `shell` in `dir` (none: inside an archive, the shell's own). `dark`: the COSMIC theme's
/// colors; `bg`: the window's background (0xAARRGGBB), the terminal's own.
pub fn start(
    tx: UnboundedSender<Event>,
    shell: &str,
    dir: Option<&Path>,
    dark: bool,
    bg: u32,
    style: &style::Style,
) -> std::io::Result<Pane> {
    terminal::WINDOW_BG_COLOR.store(bg, Ordering::SeqCst);
    let colors = if dark {
        terminal_theme::cosmic_dark()
    } else {
        terminal_theme::cosmic_light()
    };
    let options = tty::Options {
        shell: Some(tty::Shell::new(shell.to_string(), Vec::new())),
        working_directory: dir.map(Path::to_path_buf),
        ..tty::Options::default()
    };
    let config = term::Config {
        scrolling_history: HISTORY,
        ..term::Config::default()
    };
    let mut t = Terminal::new(tx, config, options, style, colors)?;
    t.set_config(style, &colors);
    Ok(Pane {
        terminal: Mutex::new(t),
        dir: dir.map(Path::to_path_buf),
        shell: shell.to_string(),
        busy_at: None,
    })
}

/// A new text size or theme for the running terminal.
pub fn restyle(pane: &Pane, style: &style::Style, dark: bool, bg: u32) {
    terminal::WINDOW_BG_COLOR.store(bg, Ordering::SeqCst);
    let colors = match dark {
        true => terminal_theme::cosmic_dark(),
        false => terminal_theme::cosmic_light(),
    };
    let mut t = pane.terminal.lock().unwrap_or_else(|e| e.into_inner());
    t.set_config(style, &colors);
}

/// What the app does about a terminal event.
pub enum Outcome {
    None,
    /// The shell ended: the pane goes (a new one starts when shown again).
    Exit,
    /// OSC 52: put this on the clipboard.
    Copy(String),
}

pub fn on_event(pane: &Pane, event: Event) -> Outcome {
    let mut t = pane.terminal.lock().unwrap_or_else(|e| e.into_inner());
    match event {
        Event::Wakeup | Event::MouseCursorDirty => t.needs_update = true,
        Event::Exit | Event::ChildExit(_) => return Outcome::Exit,
        Event::PtyWrite(text) => t.input_no_scroll(text.into_bytes()),
        Event::ColorRequest(index, f) => {
            let text = f(t.effective_color(index));
            t.input_no_scroll(text.into_bytes());
        }
        Event::TextAreaSizeRequest(f) => {
            let text = f(t.size().into());
            t.input_no_scroll(text.into_bytes());
        }
        Event::ClipboardStore(term::ClipboardType::Clipboard, data) => return Outcome::Copy(data),
        _ => {}
    }
    Outcome::None
}

/// The panel moved to `dir`: the shell follows, if it waits at its prompt (a running program is
/// never typed into) and the dir is not where it already is.
pub fn follow(pane: &mut Pane, dir: &Path) {
    if pane.dir.as_deref() == Some(dir) {
        pane.busy_at = None; // back where it is: nothing to wait for
        return;
    }
    if pane.busy_at.is_some_and(|t| t.elapsed() < RETRY) {
        return;
    }
    let t = pane.terminal.lock().unwrap_or_else(|e| e.into_inner());
    let idle = t.shell_pid().is_some_and(|pid| {
        use shagoff_core::shell::{at_prompt, canonical};
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|stat| at_prompt(&stat, canonical(pid), &pane.shell))
    });
    if idle {
        // A name that cannot be typed safely: the terminal stays where it is.
        if let Some(line) = shagoff_core::shell::cd_line(dir) {
            t.input_no_scroll(line.into_bytes());
        }
        drop(t);
        (pane.dir, pane.busy_at) = (Some(dir.to_path_buf()), None);
    } else {
        drop(t);
        pane.busy_at = Some(std::time::Instant::now());
    }
}

/// A `cd` waits for the shell's prompt: asked again every `WAIT_TICK`. The prompt's own output
/// comes before the shell takes the tty, and nothing may come after it.
pub fn waiting_tick(pane: &Pane) -> Option<Subscription<()>> {
    pane.busy_at
        .is_some()
        .then(|| cosmic::iced::time::every(WAIT_TICK).map(|_| ()))
}

/// The panel has no dir for the shell (an archive, a dir still being read): nothing to wait for.
pub fn stop_waiting(pane: &mut Pane) {
    pane.busy_at = None;
}

/// The channel terminals send their events through: its sender comes first (`TermTx`), then the
/// events (`TermEvent`).
pub fn subscription() -> Subscription<Message> {
    struct Events;
    Subscription::run_with(std::any::TypeId::of::<Events>(), |_| {
        cosmic::iced::stream::channel(100, async |mut out| {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let _ = out.send(Message::TermTx(tx)).await;
            while let Some(e) = rx.recv().await {
                if out.send(Message::TermEvent(e)).await.is_err() {
                    break;
                }
            }
            std::future::pending::<()>().await;
        })
    })
}

/// The terminal (its whole space with `full`, else `App::term_rows` rows under a top edge that is
/// dragged), or, its shell ended, a line that starts it again.
pub fn view(app: &crate::app::App, full: bool) -> cosmic::Element<'_, Message> {
    use cosmic::iced::Length;
    use cosmic::widget;
    let size = f32::from(app.config.panel_size());
    let height = match full {
        true => Length::Fill,
        false => Length::Fixed(f32::from(app.term_rows()) * (size * 1.4).ceil() + 8.0),
    };
    let inner: cosmic::Element<'_, Message> = match &app.term {
        Some(p) => terminal_box::terminal_box(&p.terminal, pass)
            .id(app.term_id.clone())
            .on_focus(Message::TermFocused)
            .padding(4)
            .into(),
        // Anywhere in its space: what was the terminal stays the place to click.
        None => widget::mouse_area(
            widget::container(widget::text::body(crate::fl!("term-ended")))
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::TermStart)
        .into(),
    };
    let body = widget::container(inner)
        .width(Length::Fill)
        .height(height)
        .padding([0, 6]);
    if full {
        return body.into();
    }
    // The top edge: a few pixels the pointer can hold (as the panes' divider).
    let edge = widget::mouse_area(
        widget::container(widget::divider::horizontal::default())
            .padding([3, 0])
            .width(Length::Fill),
    )
    .interaction(cosmic::iced::mouse::Interaction::ResizingVertically)
    .on_press(Message::TermDragStart);
    widget::column![edge, body].into()
}

/// Keys the terminal leaves to the app, on either layout: Ctrl+O (whole height / back) and Ctrl+`
/// (hide it).
pub fn pass(mods: Modifiers, key: &Key) -> bool {
    mods.control()
        && !mods.shift()
        && !mods.alt()
        && matches!(key.as_ref(), Key::Character(c) if matches!(c, "o" | "O" | "щ" | "Щ" | "`" | "ё" | "Ё"))
}
