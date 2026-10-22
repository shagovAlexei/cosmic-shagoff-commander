//! The command line, the built-in terminal, tab targets and scroll restore.

use super::*;

impl App {
    /// The command line takes input: shown, and nothing modal or in front of the panels.
    pub(super) fn cmd_ready(&self) -> bool {
        self.config.show_cmdline
            && self.dialog.is_none()
            && self.drawer.is_none()
            && !self.viewer()
            && self.search.is_none()
            && !self.busy()
    }

    /// Num+ / Num− and "Open with command": ↑ older entry, ↓ newer (TC's drop-down history).
    pub(super) fn mask_history(&mut self, action: Action) -> Option<Task<Message>> {
        let (list, input) = match &mut self.dialog {
            Some(Dialog::Mask { input, .. }) => (&self.masks, input),
            Some(Dialog::Input {
                op: InputOp::OpenWith,
                input,
                ..
            }) => (&self.other_cmds, input),
            _ => return None,
        };
        let step = match action {
            Action::Up => cmdline::previous(list, input),
            Action::Down => cmdline::next(list, input),
            _ => return None,
        };
        if let Some(m) = step {
            *input = m;
        }
        Some(widget::text_input::move_cursor_to_end(
            self.input_id.clone(),
        ))
    }

    /// A job's dialog is up: the panels wait. A job in the background leaves them free.
    pub(super) fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.hidden)
    }

    /// Command line keys and the panel keys it changes (Enter, Space with text typed).
    pub(super) fn cmd_action(&mut self, side: usize, action: Action) -> Option<Task<Message>> {
        if !self.cmd_ready() {
            return matches!(
                action,
                Action::CmdName
                    | Action::CmdPath
                    | Action::CmdCwd
                    | Action::CmdPrevious
                    | Action::CmdHistory
            )
            .then(Task::none);
        }
        let panel = &self.panes[side].active().panel;
        // The names in `dir` (relative: under the panel's dir), for a name not in UTF-8
        // (`cmdline::word`). Read, not the panel's list: a filter hides some, search results hold
        // paths. Only for such names, so rarely.
        let siblings = |dir: &Path| -> Option<Vec<OsString>> {
            let rd = std::fs::read_dir(panel.cwd().join(dir)).ok()?;
            Some(rd.flatten().map(|e| e.file_name()).collect())
        };
        let word = match action {
            Action::Mark if !self.cmdline.is_empty() => {
                self.cmdline.push(' ');
                None
            }
            Action::CmdName | Action::CmdPath | Action::CmdCwd => {
                let path = match action {
                    Action::CmdCwd => panel.cwd().to_path_buf(),
                    _ => {
                        let e = panel.current().filter(|e| e.name != PARENT)?;
                        match action {
                            Action::CmdPath => panel.cwd().join(&e.os_name),
                            // Search results / branch: the path below the root, as shown.
                            _ => Path::new(&e.os_name)
                                .strip_prefix(panel.cwd())
                                .unwrap_or(Path::new(&e.os_name))
                                .to_path_buf(),
                        }
                    }
                };
                let Some(w) = cmdline::word(&path, siblings) else {
                    self.say(StatusKind::Error, fl!("cmd-name-ambiguous"));
                    return Some(Task::none());
                };
                Some(w)
            }
            Action::CmdPrevious => {
                self.cmdline = cmdline::previous(&self.commands, &self.cmdline)?;
                None
            }
            Action::CmdHistory => {
                if !self.commands.is_empty() {
                    self.dialog = Some(Dialog::List {
                        kind: ListKind::Commands,
                        side,
                        cursor: 0,
                        items: (self.commands.iter())
                            .map(|c| ListItem {
                                label: c.clone(),
                                path: PathBuf::new(),
                                kind: Item::Dir,
                                icon: String::new(),
                            })
                            .collect(),
                    });
                }
                return Some(Task::none());
            }
            _ => return None,
        };
        if let Some(w) = word {
            self.cmdline = cmdline::append(&self.cmdline, &w);
        }
        Some(self.cmd_focus())
    }

    /// Focus the command line with the cursor at the end (an already focused field keeps its cursor).
    pub(super) fn cmd_focus(&self) -> Task<Message> {
        Task::batch([
            widget::text_input::focus(self.cmd_id.clone()),
            widget::text_input::move_cursor_to_end(self.cmd_id.clone()),
        ])
    }

    /// Run the command line in the active panel's dir (`cd` changes the dir instead).
    pub(super) fn cmd_run(&mut self, terminal: bool) -> Task<Message> {
        if !self.cmd_ready() {
            return Task::none();
        }
        let line = std::mem::take(&mut self.cmdline);
        let side = self.active;
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let Some(cmd) = cmdline::parse(&line, &cwd, &self.home) else {
            return unfocus();
        };
        self.commands = cmdline::remember(&self.commands, &line);
        let task = match cmd {
            Cmd::Cd(dir) => self.go_to(side, dir),
            Cmd::Run(line) => {
                let prefix = self.terminal_prefix();
                let term = terminal.then_some(prefix.as_slice());
                if let Err(e) = spawn_in(&cmdline::argv(&line, &cwd, term), &cwd) {
                    self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                }
                Task::none()
            }
        };
        Task::batch([unfocus(), task])
    }

    /// The terminal chosen in Commands → Terminal, else the system's default.
    pub fn terminal(&self) -> Option<&Terminal> {
        let id = &self.config.terminal_app;
        (self
            .terminals
            .iter()
            .find(|t| !id.is_empty() && t.id == *id))
        .or_else(|| self.term_default.and_then(|i| self.terminals.get(i)))
    }

    /// The dir the terminal follows: the active panel's, on disk (not inside an archive).
    pub(super) fn term_dir(&self) -> Option<PathBuf> {
        let t = self.panes[self.active].active();
        let cwd = t.panel.cwd();
        (t.pending.is_none() && !archive::within(cwd)).then(|| cwd.to_path_buf())
    }

    /// Start the built-in terminal's shell, if it is on, its channel is up and none runs.
    pub(super) fn term_start(&mut self) {
        let Some(tx) = self.term_tx.clone().filter(|_| self.config.term_builtin) else {
            return;
        };
        // Not before the panel's dir is read: the shell starts there (a `cd` typed while it
        // still starts would show before its prompt).
        let Some(dir) = self.term_dir().filter(|_| self.term.is_none()) else {
            return;
        };
        let shell = shagoff_core::shell::pick(
            &self.config.term_shell,
            std::env::var("SHELL").ok().as_deref(),
        );
        let (style, dark, bg) = self.term_look();
        match crate::term::start(tx, &shell, Some(&dir), dark, bg, &style) {
            Ok(pane) => self.term = Some(Box::new(pane)),
            // Not again on every message: Ctrl+O or a click on its line tries again.
            Err(e) => {
                self.term_ended = true;
                self.say(
                    StatusKind::Error,
                    fl!("term-failed", err = crate::why::text(&e)),
                );
            }
        }
    }

    /// The terminal's text size (the panels'), and the theme: dark or not, its background.
    pub(super) fn term_look(&self) -> (crate::term::style::Style, bool, u32) {
        let theme = cosmic::theme::active();
        let c = theme.cosmic();
        let bg = c.bg_color();
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let bg = 0xFF00_0000 | byte(bg.red) << 16 | byte(bg.green) << 8 | byte(bg.blue);
        let style = crate::term::style::Style {
            font_size: f32::from(self.config.panel_size()),
            ..Default::default()
        };
        (style, c.is_dark, bg)
    }

    /// The running terminal takes the panels' text size and the theme now.
    pub(super) fn term_restyle(&mut self) {
        let (style, dark, bg) = self.term_look();
        if let Some(pane) = &self.term {
            crate::term::restyle(pane, &style, dark, bg);
        }
    }

    /// Rows under the panels: dragged now, else the setting.
    pub fn term_rows(&self) -> u16 {
        match self.term_drag {
            Some(_) => self.term_rows,
            None => self.config.term_rows,
        }
    }

    /// After every `update`: the shell starts once the panel's dir is read, then goes where the
    /// active panel is (`term::follow`).
    pub(super) fn term_follow(&mut self) {
        if self.term.is_none() {
            if !self.term_ended && self.term_tx.is_some() {
                self.term_start();
            }
            return;
        }
        let dir = self.term_dir();
        if let Some(pane) = &mut self.term {
            match dir {
                Some(dir) => crate::term::follow(pane, &dir),
                None => crate::term::stop_waiting(pane),
            }
        }
    }

    /// Commands → "Shell…": `$SHELL` first, then `/etc/shells`; the chosen one ticked.
    pub(super) fn shell_items(&self) -> (Vec<ListItem>, usize) {
        let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let list = shagoff_core::shell::shells(&etc);
        let chosen = &self.config.term_shell;
        let at = list.iter().position(|s| s == chosen);
        let env = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let tick = |on: bool, label: String| format!("{}{label}", if on { "✓ " } else { "    " });
        let items = std::iter::once(ListItem {
            label: tick(chosen.is_empty(), fl!("menu-shell-default", name = env)),
            path: PathBuf::new(),
            kind: Item::App,
            icon: String::new(),
        })
        .chain(list.iter().enumerate().map(|(i, s)| ListItem {
            label: tick(at == Some(i), s.clone()),
            path: PathBuf::from(s),
            kind: Item::App,
            icon: String::new(),
        }))
        .collect();
        (items, at.map_or(0, |i| i + 1))
    }

    /// Commands → "Terminal…": the default first, then the installed ones; the chosen one ticked
    /// and under the cursor.
    pub(super) fn terminal_items(&self) -> (Vec<ListItem>, usize) {
        let id = &self.config.terminal_app;
        let chosen = (self.terminals.iter()).position(|t| !id.is_empty() && t.id == *id);
        let default = match self.term_default.and_then(|i| self.terminals.get(i)) {
            Some(t) => fl!("menu-terminal-default", name = t.name.clone()),
            None => fl!("menu-terminal-none"),
        };
        let tick = |on: bool, label: String| format!("{}{label}", if on { "✓ " } else { "    " });
        let items = std::iter::once(ListItem {
            label: tick(chosen.is_none(), default),
            path: PathBuf::new(),
            kind: Item::App,
            icon: String::new(),
        })
        .chain(self.terminals.iter().enumerate().map(|(i, t)| ListItem {
            label: tick(chosen == Some(i), t.name.clone()),
            path: PathBuf::from(&t.id),
            kind: Item::App,
            icon: String::new(),
        }))
        .collect();
        (items, chosen.map_or(0, |i| i + 1))
    }

    /// Shift+Enter's terminal: `terminal` changed by hand wins (older setups), then the chosen one.
    pub(super) fn terminal_prefix(&self) -> Vec<String> {
        let by_hand = self.config.terminal != Config::default().terminal;
        match self.terminal().filter(|_| !by_hand) {
            Some(t) => t.run_prefix(),
            None => self.config.terminal.clone(),
        }
    }

    /// Commands → Open terminal: a window in the active panel's dir (inside an archive: the
    /// archive's dir, the one on disk).
    pub(super) fn open_terminal(&mut self) {
        let cwd = self.panes[self.active].active().panel.cwd().to_path_buf();
        let dir = archive::split_path(&cwd)
            .and_then(|(a, _)| a.parent().map(Path::to_path_buf))
            .unwrap_or(cwd);
        let argv = match self.terminal() {
            Some(t) => t.argv.clone(),
            None => self.config.terminal.iter().take(1).cloned().collect(),
        };
        if let Err(e) = spawn_in(&argv, &dir) {
            let error = format!("{}: {e}", argv.first().map_or("", String::as_str));
            self.say(StatusKind::Error, fl!("cmd-failed", error = error));
        }
    }

    /// Ctrl+↑: the dir under the cursor (an archive as a dir, ".." the parent), else the panel's own.
    pub(super) fn tab_target(&self, side: usize) -> PathBuf {
        let t = self.panes[side].active();
        let panel = &t.panel;
        // ".." in search results / a branch leads back to the dir they are of (as Backspace).
        if t.results.is_some() && panel.current().is_some_and(|e| e.name == PARENT) {
            return panel.cwd().to_path_buf();
        }
        if let Some((path, _)) = panel.enter_path() {
            return path;
        }
        let cwd = panel.cwd();
        match panel.current() {
            Some(e)
                if archive::split_path(cwd).is_none()
                    && Format::detect(&e.name).is_some_and(Format::is_tree) =>
            {
                cwd.join(&e.os_name)
            }
            _ => cwd.to_path_buf(),
        }
    }

    pub(super) fn go_to(&mut self, side: usize, path: PathBuf) -> Task<Message> {
        self.active = side;
        self.load(side, path, None)
    }

    /// The newly shown tab was not watched while hidden: restore its scroll and rescan it.
    pub(super) fn tab_switched(&mut self, side: usize) -> Task<Message> {
        let t = self.panes[side].active();
        let (dir, tab) = (t.target(), t.id);
        // A hidden Brief / Thumbnails tab missed the resizes: regroup its columns.
        let reveal = if t.brief || t.thumbs {
            self.reveal(side, tab)
        } else {
            Task::none()
        };
        Task::batch([
            reveal,
            self.restore_scroll(side),
            self.reload(side, dir, None),
        ])
    }

    /// What the viewer covers: none (also in a window of its own), both panes (F3 in place) or
    /// the pane opposite the active one (Ctrl+Q).
    pub(super) fn viewer_shown(&self) -> Option<Option<usize>> {
        let l = (self.lister.as_deref()).filter(|l| l.quick.is_some() || l.in_place())?;
        Some(l.quick.is_some().then_some(1 - self.active))
    }

    /// A right press while the right button marks: held for `HOLD` it arms the menu.
    pub(super) fn right_press(&mut self, side: usize, row: Option<usize>) -> Task<Message> {
        const HOLD: std::time::Duration = std::time::Duration::from_millis(500);
        self.right_presses += 1;
        let n = self.right_presses;
        let tab = self.panes[side].active().id;
        self.right_hold = Some(Hold {
            n,
            side,
            tab,
            row,
            armed: false,
            drag: None,
        });
        Task::perform(async { tokio::time::sleep(HOLD).await }, move |()| {
            cosmic::Action::App(Message::RightHeld(n))
        })
    }

    /// Scroll the pane's list to the active tab's stored offset.
    pub(super) fn restore_scroll(&self, side: usize) -> Task<Message> {
        scrollable::scroll_to(
            self.scroll_ids[side].clone(),
            AbsoluteOffset {
                x: None,
                y: Some(self.panes[side].active().offset),
            },
        )
    }
}
