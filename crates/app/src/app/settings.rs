//! Settings and saved lists: `set` / `apply_config`, mounts, «Open with», hotlist, toolbar.

use super::*;

impl App {
    /// A change in the settings drawer: written like a hand edit of the config, applied now.
    pub(super) fn set(&mut self, s: Setting) -> Task<Message> {
        let mut c = self.config.clone();
        let form = match &mut self.drawer {
            Some(Drawer::Settings(f)) => Some(f),
            _ => None,
        };
        let command = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        match s {
            Setting::Language(i) => {
                c.language = i
                    .checked_sub(1)
                    .and_then(|i| config::LANGUAGES.get(i))
                    .map_or(String::new(), |l| l.to_string());
            }
            Setting::TerminalApp(id) => c.terminal_app = id,
            Setting::TermBuiltin(b) => c.term_builtin = b,
            Setting::TermShell(s) => c.term_shell = s,
            Setting::TermRows(n) => c.term_rows = n,
            Setting::Theme(i) => c.app_theme = config::AppTheme::ALL[i.min(2)],
            Setting::Skin(i) => c.skin = config::Skin::ALL[i.min(1)],
            Setting::ShowFkeys(b) => c.show_fkeys = b,
            Setting::DividerLine(b) => c.divider_line = b,
            Setting::HumanSizes(b) => c.human_sizes = Some(b),
            Setting::ActiveFrame(b) => c.active_frame = b,
            Setting::ShowCmdline(b) => c.show_cmdline = b,
            Setting::KeepHistory(b) => c.keep_history = b,
            Setting::InternalViewer(b) => c.internal_viewer = b,
            Setting::ShowHidden(b) => c.show_hidden = b,
            Setting::RightMarks(b) => c.right_marks = b,
            Setting::ListerWindow(b) => c.lister_window = b,
            // 0: own program (typed below), else `builtins()[i - 1]`.
            Setting::F12(0) => {
                if crate::toolbar::builtin(&c.f12.cmd).is_some() {
                    c.f12 = ToolButton::default();
                }
            }
            Setting::F12(i) => {
                if let Some(b) = crate::toolbar::builtins().get(i - 1) {
                    c.f12 = ToolButton {
                        cmd: b.cm.into(),
                        ..ToolButton::default()
                    };
                }
            }
            Setting::F12Cmd(s) => c.f12.cmd = s,
            Setting::F12Tip(s) => c.f12.tip = s,
            Setting::F12Params(s) => c.f12.params = s,
            Setting::LastTabHome(b) => {
                c.last_tab_close = if b { LastTab::Home } else { LastTab::Nothing };
            }
            Setting::HomeDir(v) => {
                c.home_dir = (!v.trim().is_empty()).then(|| PathBuf::from(v.trim()));
                if let Some(f) = form {
                    f.home = v;
                }
            }
            Setting::Viewer(v) => {
                c.viewer = command(&v);
                if let Some(f) = form {
                    f.viewer = v;
                }
            }
            Setting::Editor(v) => {
                c.editor = command(&v);
                if let Some(f) = form {
                    f.editor = v;
                }
            }
            Setting::PanelFont(i) => {
                c.panel_font = match &self.drawer {
                    Some(Drawer::Settings(f)) => i
                        .checked_sub(1)
                        .and_then(|i| f.fonts.get(i))
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            }
            Setting::PanelSize(s) => c.panel_size = s,
            Setting::ListerSize(s) => c.lister_size = s,
            Setting::PackFormat(i) => {
                if let Some(f) = Format::PACK.get(i) {
                    c.pack_format = f.ext().into();
                }
            }
        }
        if let Some(h) = &self.config_handler
            && let Err(e) = c.write_entry(h)
        {
            self.config_not_saved(&e);
        }
        self.apply_config(c)
    }

    /// New settings, from the settings drawer or the config watcher (a hand edit).
    pub(super) fn apply_config(&mut self, c: Config) -> Task<Message> {
        let old = std::mem::replace(&mut self.config, c);
        if old.hotkeys != self.config.hotkeys {
            keymap::set_own(crate::hotkeys::resolve(&self.config.hotkeys));
        }
        self.scheme =
            shagoff_core::colors::Scheme::new(&self.config.colors).with_panel(&self.config.panel);
        self.cols = self.config.columns;
        let mut tasks = Vec::new();
        if old.language != self.config.language {
            let system = i18n_embed::DesktopLanguageRequester::requested_languages();
            crate::i18n::init(&config::languages(&self.config.language, system));
            self.core.window.header_title = fl!("app-title");
            self.about = drawer::about();
        }
        if old.app_theme != self.config.app_theme {
            tasks.push(cosmic::command::set_theme(self.config.app_theme.theme()));
        }
        // Off by any path (the menu, a hand edit): the shell ends with its terminal.
        if !self.config.term_builtin {
            (self.term, self.term_tx, self.term_full, self.term_focused) =
                (None, None, false, false);
        }
        // A new text size or theme: the running terminal too.
        if old.panel_size() != self.config.panel_size() || old.app_theme != self.config.app_theme {
            self.term_restyle();
        }
        if old.show_hidden != self.config.show_hidden {
            tasks.push(self.apply_hidden());
        }
        if old.panel_font != self.config.panel_font {
            self.face = font_named(&self.config.panel_font);
        }
        // Changed under the open Ctrl+D (by hand, another window): its rows are indices into the
        // list, so they are rebuilt, the cursor kept on the same entry.
        if old.hotlist != self.config.hotlist
            && let Some(Dialog::List {
                kind: ListKind::Hotlist,
                items,
                cursor,
                ..
            }) = &self.dialog
        {
            let was = items
                .get(*cursor)
                .map(|it| (it.label.clone(), it.path.clone()));
            let at = *cursor;
            self.hotlist_refresh(at);
            if let (Some(was), Some(Dialog::List { items, cursor, .. })) = (was, &mut self.dialog)
                && let Some(i) = items
                    .iter()
                    .position(|it| (&it.label, &it.path) == (&was.0, &was.1))
            {
                *cursor = i;
            }
        }
        if let Some(l) = &mut self.lister
            && l.size != self.config.lister_size()
        {
            // The same top row at the new row height, the wrapped lines rewrapped.
            let row = l.offset.1 / l.row_h();
            l.set_size(self.config.lister_size());
            let (x, y) = (l.offset.0, row * l.row_h());
            tasks.push(lister_scroll(l, x, y));
        }
        if old.row_h() != self.config.row_h() {
            // Offsets are in pixels of the old row height: bring each cursor back into view.
            let scale = self.config.row_h() / old.row_h();
            let cells =
                crate::thumbs::cell_h(self.config.row_h()) / crate::thumbs::cell_h(old.row_h());
            for side in 0..2 {
                // Hidden tabs too: they are shown with their stored offset.
                for t in self.panes[side].items_mut() {
                    t.offset *= if t.thumbs { cells } else { scale };
                }
                let tab = self.panes[side].active().id;
                tasks.push(self.reveal(side, tab));
                tasks.push(self.restore_scroll(side));
            }
        }
        Task::batch(tasks)
    }

    /// Mounts change rarely and procfs never blocks: re-read with every listing.
    /// gvfs network mounts are dirs under `$XDG_RUNTIME_DIR/gvfs`, not separate mounts: that dir
    /// blocks while one of them hangs, so it is read in the background (`read_gvfs`).
    pub(super) fn refresh_mounts(&mut self) {
        self.gvfs_want = true;
        self.set_drives();
    }

    pub(super) fn set_drives(&mut self) {
        if let Ok(m) = std::fs::read_to_string("/proc/self/mounts") {
            self.drives = drives::parse(&m, &self.home);
        }
        self.drives.extend(self.gvfs.iter().cloned());
    }

    /// Run after every `update`: the gvfs dir read asked for by `refresh_mounts`, if none is
    /// under way (a hung one would otherwise pile up threads).
    pub(super) fn read_gvfs(&mut self) -> Task<Message> {
        if !self.gvfs_want || self.gvfs_busy {
            return Task::none();
        }
        let Some(root) = mount::gvfs_root() else {
            return Task::none();
        };
        (self.gvfs_want, self.gvfs_busy) = (false, true);
        Task::perform(
            async move {
                let drives = tokio::task::spawn_blocking(move || mount::gvfs_drives(&root));
                Message::Gvfs(drives.await.unwrap_or_default())
            },
            cosmic::Action::App,
        )
    }

    /// Ctrl+Shift+F: unmount the removable or network drive holding the active panel. Not while a
    /// job runs: it may be copying from or to that drive.
    pub(super) fn disconnect(&mut self, side: usize) -> Task<Message> {
        let cwd = self.panes[side].active().target();
        let gvfs = mount::gvfs_root();
        let Some(i) = drives::containing(&self.drives, &cwd)
            .filter(|&i| mount::removable(&self.drives[i].path, gvfs.as_deref()))
        else {
            return Task::none();
        };
        if self.job.is_some() {
            self.say(StatusKind::Error, fl!("job-running")); // it may be using that disk
            return Task::none();
        }
        let root = self.drives[i].path.clone();
        self.say(StatusKind::Busy, fl!("unmounting"));
        let r = root.clone();
        blocking(
            move || mount::unmount(&r),
            move |res| Message::Unmounted(root.clone(), res),
        )
    }

    /// Context menu "Open with…": the programs for the file under the cursor, found off the UI
    /// thread (`gio` reads the file).
    pub(super) fn open_with(&mut self, side: usize) -> Task<Message> {
        let panel = &self.panes[side].active().panel;
        let Some(file) = (panel.current())
            .filter(|e| !e.is_dir() && e.name != PARENT)
            .map(|e| panel.cwd().join(&e.os_name))
        else {
            return Task::none();
        };
        if inside_archive(&file) {
            self.say(StatusKind::Error, fl!("open-with-archive"));
            return Task::none();
        }
        use i18n_embed::LanguageLoader;
        let files = panel.targets();
        let lang = crate::i18n::LANGUAGE_LOADER
            .current_language()
            .language
            .to_string();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || openwith::apps(&file, &lang))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::OpenWithApps(side, files, r)),
        )
    }

    /// Enter / click on entry `i` of the open list.
    pub(super) fn pick(&mut self, i: usize) -> Task<Message> {
        // Into a submenu or back out of it: the menu stays open.
        if let Some(Dialog::List { items, .. }) = &self.dialog {
            match items.get(i).map(|it| it.kind) {
                Some(Item::Sub(s)) => {
                    self.hot_sub.push(s);
                    self.hotlist_refresh(1); // past "‹ back"
                    return Task::none();
                }
                Some(Item::Back) => {
                    self.hotlist_back();
                    return Task::none();
                }
                Some(Item::All) => {
                    use i18n_embed::LanguageLoader;
                    let lang = crate::i18n::LANGUAGE_LOADER
                        .current_language()
                        .language
                        .to_string();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                openwith::all_apps(&openwith::data_dirs(), &lang)
                            })
                            .await
                            .unwrap_or_default()
                        },
                        |apps| cosmic::Action::App(Message::OpenWithAll(apps)),
                    );
                }
                _ => {}
            }
        }
        let Some(Dialog::List {
            kind, side, items, ..
        }) = self.dialog.take()
        else {
            return Task::none();
        };
        if kind == ListKind::OpenWith {
            let files = std::mem::take(&mut self.open_with);
            if items.get(i).is_some_and(|it| it.kind == Item::Other) {
                self.dialog = Some(Dialog::Input {
                    op: InputOp::OpenWith,
                    side,
                    sources: files,
                    input: self.other_cmds.first().cloned().unwrap_or_default(),
                });
                return widget::text_input::focus(self.input_id.clone());
            }
            if let Some(item) = items.get(i)
                && let Err(e) = spawn_detached(&openwith::launch_argv(&item.path, &files))
            {
                self.say(
                    StatusKind::Error,
                    fl!("open-failed", err = crate::why::text(&e)),
                );
            }
            return Task::none();
        }
        if kind == ListKind::Context {
            use cosmic::widget::menu::Action as _;
            return match items.get(i).map(|it| it.kind) {
                Some(Item::Act(act)) => self.handle(act.message()),
                _ => Task::none(),
            };
        }
        if kind == ListKind::Templates {
            let Some(item) = items.into_iter().nth(i) else {
                return Task::none();
            };
            self.dialog = Some(Dialog::Input {
                op: InputOp::FromTemplate,
                side,
                input: item.label,
                sources: vec![item.path],
            });
            return widget::text_input::focus(self.input_id.clone());
        }
        if kind == ListKind::Tabs {
            self.panes[side].select(i);
            return self.tab_switched(side);
        }
        if kind == ListKind::NameEncoding {
            let (Some(it), Some(&enc)) = (items.get(i), archive::NameEnc::ALL.get(i)) else {
                return Task::none();
            };
            archive::set_name_encoding(&it.path, enc);
            // Every tab inside it lists it anew (the index is keyed by the encoding), from its
            // root: the dirs' names change too.
            let mut tasks = Vec::new();
            for s in 0..2 {
                for t in 0..self.panes[s].items().len() {
                    if self.panes[s].items()[t].target().starts_with(&it.path) {
                        tasks.push(self.load_tab(s, t, it.path.clone(), None));
                    }
                }
            }
            return Task::batch(tasks);
        }
        if kind == ListKind::Shells {
            let path = items
                .get(i)
                .map(|it| it.path.to_string_lossy().into_owned());
            let Some(path) = path.filter(|p| *p != self.config.term_shell) else {
                return Task::none();
            };
            // A new shell from now on: the running one ends with its terminal.
            self.term = None;
            let task = self.set(Setting::TermShell(path));
            self.term_start();
            return task;
        }
        if kind == ListKind::Terminals {
            let id = items
                .get(i)
                .map(|it| it.path.to_string_lossy().into_owned());
            return id.map_or(Task::none(), |id| self.set(Setting::TerminalApp(id)));
        }
        if kind == ListKind::Commands {
            if let Some(item) = items.get(i) {
                self.cmdline.clone_from(&item.label);
                return self.cmd_focus();
            }
            return Task::none();
        }
        let what = items.get(i).map(|it| it.kind);
        if what == Some(Item::Configure) {
            let list = self.config.hotlist.clone();
            self.dialog = Some(Dialog::Hotlist(Box::new(HotEdit::new(side, list))));
            return Task::none();
        }
        if what == Some(Item::Sep) {
            return Task::none();
        }
        if what == Some(Item::Add) {
            let cwd = self.panes[side].active().panel.cwd().to_path_buf();
            // The config is text (RON): a path that is not UTF-8 can't be written to it.
            if cwd.to_str().is_none() {
                self.dialog = None;
                self.say(StatusKind::Error, fl!("hotlist-not-utf8"));
                return Task::none();
            }
            if !self.config.hotlist.iter().any(|e| e.path == cwd) {
                let mut list = self.config.hotlist.clone();
                // Into the submenu shown, before its `--`.
                let at = (self.hot_sub.last()).map_or(list.len(), |&s| hotlist::sub_end(&list, s));
                list.insert(
                    at,
                    HotEntry {
                        name: hotlist::menu_name(&cwd),
                        path: cwd,
                    },
                );
                self.save_hotlist(list);
            }
            return Task::none();
        }
        match items.get(i) {
            Some(item) if item.kind == Item::Mount => {
                self.say(StatusKind::Busy, fl!("mounting"));
                let device = item.path.to_string_lossy().into_owned();
                blocking(
                    move || mount::mount_device(&device),
                    move |r| Message::Mounted(side, r),
                )
            }
            Some(item) => self.go_to(side, item.path.clone()),
            None => Task::none(),
        }
    }

    /// Hotlist rows as TC's menu, for the submenu open (`hot_sub`): "‹ back" in a submenu, its
    /// favourites and submenus, a line, "add current dir", "configure…".
    pub(super) fn hotlist_items(&self) -> Vec<ListItem> {
        let own = |label, kind| ListItem {
            label,
            path: PathBuf::new(),
            kind,
            icon: String::new(),
        };
        let list = &self.config.hotlist;
        // A stale index (the config changed under the open menu): back to the top.
        let open = self
            .hot_sub
            .last()
            .copied()
            .filter(|&s| list.get(s).is_some_and(hotlist::is_sub));
        let mut items: Vec<ListItem> = Vec::new();
        if let Some(s) = open {
            items.push(own(
                // No hot letter of its own: the submenu's would take it from the items inside.
                format!("‹ {}", hotkey_free(hotlist::sub_name(&list[s]))),
                Item::Back,
            ));
        }
        items.extend(hotlist::children(list, open).into_iter().map(|i| {
            let e = &list[i];
            match () {
                _ if hotlist::is_sep(e) => own(String::new(), Item::Sep),
                _ if hotlist::is_sub(e) => own(hotlist::sub_name(e).into(), Item::Sub(i)),
                _ => ListItem {
                    label: e.name.clone(),
                    path: e.path.clone(),
                    kind: Item::Hot(i),
                    icon: String::new(),
                },
            }
        }));
        if !items.is_empty() {
            items.push(own(String::new(), Item::Sep));
        }
        items.push(own(fl!("hotlist-add"), Item::Add));
        items.push(own(fl!("hotlist-configure"), Item::Configure));
        items
    }

    pub(super) fn save_pack_format(&mut self, f: Format) {
        let v = f.ext().to_string();
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_pack_format(h, v) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.pack_format = v,
        }
    }

    /// The widths on screen into the config, if they changed.
    pub(super) fn save_columns(&mut self) {
        if self.cols == self.config.columns {
            return;
        }
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_columns(h, self.cols) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.columns = self.cols,
        }
    }

    /// The held toolbar button let go: the bar saved if it moved.
    pub(super) fn tool_drop(&mut self) {
        if self.tool_drag.release() {
            // Moved in place: taken out, or the setter would see no change to write.
            let list = std::mem::take(&mut self.config.toolbar);
            self.save_toolbar(list);
        }
    }

    /// `rows` of `panel` as `format::details_table`: the panel's columns, a dir's counted size.
    pub(super) fn details_text(&self, panel: &Panel, rows: &[&Entry], head: bool) -> String {
        let rows: Vec<(&Entry, String)> = (rows.iter())
            .map(|&e| (e, crate::view::size_cell(self, panel, e)))
            .collect();
        let titles = [
            fl!("col-name"),
            fl!("col-ext"),
            fl!("col-size"),
            fl!("col-date"),
            fl!("col-attr"),
        ];
        let titles = titles.each_ref().map(String::as_str);
        format::details_table(&rows, &self.tz, head.then_some(titles))
    }

    pub(super) fn save_toolbar(&mut self, list: Vec<ToolButton>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_toolbar(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.toolbar = list,
        }
    }

    /// A toolbar button: its internal command, or its program with the parameters filled in.
    pub(super) fn run_tool(&mut self, i: usize) -> Task<Message> {
        match self.config.toolbar.get(i) {
            Some(b) => self.run_button(b.clone()),
            None => Task::none(),
        }
    }

    /// A toolbar button or F12: its internal command, or its program with the parameters expanded.
    pub(super) fn run_button(&mut self, b: ToolButton) -> Task<Message> {
        if let Some(c) = crate::toolbar::builtin(&b.cmd) {
            return self.update(Message::Key(c.action));
        }
        if b.cmd.trim().is_empty() || b.cmd == crate::toolbar::SEP {
            return Task::none();
        }
        let (panel, other) = (
            &self.panes[self.active].active().panel,
            &self.panes[1 - self.active].active().panel,
        );
        let cwd = panel.cwd().to_path_buf();
        let selected: Vec<String> = (panel.targets().iter())
            .map(|p| p.strip_prefix(&cwd).unwrap_or(p).display().to_string())
            .collect();
        let vars = cmdline::Vars {
            dir: &cwd,
            other: other.cwd(),
            name: panel
                .current()
                .map(|e| e.name.as_str())
                .filter(|&n| n != ".."),
            selected: &selected,
        };
        let line = cmdline::append(&b.cmd, &cmdline::expand(&b.params, &vars));
        let home = std::env::home_dir().unwrap_or_else(|| "/".into());
        let dir = cmdline::start_dir(&b.dir, &vars, &home);
        match run_watched(&cmdline::argv(&line, &dir, None), &dir) {
            Ok(task) => task,
            Err(e) => {
                self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                Task::none()
            }
        }
    }

    /// A setting could not be written: it holds until exit only — said, not just logged.
    pub(super) fn config_not_saved(&mut self, e: &dyn std::fmt::Display) {
        log::warn!("config: {e}");
        self.say(
            StatusKind::Error,
            fl!("config-not-saved", err = e.to_string()),
        );
    }

    pub(super) fn save_hotlist(&mut self, list: Vec<HotEntry>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_hotlist(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.hotlist = list,
        }
    }

    pub(super) fn save_connections(&mut self, list: Vec<String>) {
        match &self.config_handler {
            Some(h) => {
                if let Err(e) = self.config.set_connections(h, list) {
                    self.config_not_saved(&e);
                }
            }
            None => self.config.connections = list,
        }
    }

    /// Rebuild the open hotlist menu (after a submenu change or an edit), cursor at `cursor`
    /// or the next row that is not a line.
    pub(super) fn hotlist_refresh(&mut self, cursor: usize) {
        let fresh = self.hotlist_items();
        if let Some(Dialog::List {
            items, cursor: c, ..
        }) = &mut self.dialog
        {
            let i = cursor.min(fresh.len() - 1);
            *c = (i..fresh.len())
                .find(|&j| fresh[j].kind != Item::Sep)
                .unwrap_or(i);
            *items = fresh;
        }
    }

    /// Out of the open submenu, the cursor on it (as a TC menu).
    pub(super) fn hotlist_back(&mut self) {
        let left = self.hot_sub.pop();
        self.hotlist_refresh(0);
        if let Some(Dialog::List { items, cursor, .. }) = &mut self.dialog
            && let Some(i) = items
                .iter()
                .position(|it| Some(it.kind) == left.map(Item::Sub))
        {
            *cursor = i;
        }
    }

    /// Delete on hotlist entry `i`: drop it, save, refresh the open list in place.
    pub(super) fn hotlist_remove(&mut self, i: usize) -> Task<Message> {
        let mut list = self.config.hotlist.clone();
        if i < list.len() {
            list.remove(i);
            self.save_hotlist(list);
        }
        let at = match &self.dialog {
            Some(Dialog::List { cursor, .. }) => *cursor,
            _ => 0,
        };
        self.hotlist_refresh(at);
        Task::none()
    }
}
