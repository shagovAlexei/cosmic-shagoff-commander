//! Tabs and directory changes: `load` / `reload` / `load_tab`, locked tabs, the Ctrl+F8 tree.

use super::*;

impl App {
    pub(super) fn new_tab(&mut self, path: PathBuf) -> Tab {
        let mut t = Tab::new(self.next_id(), path);
        t.panel.set_show_hidden(self.config.show_hidden);
        t
    }

    /// Rescan every tab of both panes (startup, Ctrl+H).
    pub(super) fn load_all(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for side in 0..2 {
            for i in 0..self.panes[side].items().len() {
                let dir = self.panes[side].items()[i].target();
                tasks.push(self.load_tab(side, i, dir, None));
            }
        }
        Task::batch(tasks)
    }

    pub(super) fn apply_hidden(&mut self) -> Task<Message> {
        let on = self.config.show_hidden;
        for side in 0..2 {
            for t in self.panes[side].items_mut() {
                t.panel.set_show_hidden(on);
            }
        }
        self.load_all()
    }

    /// Write tab paths when they changed (cheap compare on every message; no write if equal).
    pub(super) fn save_state(&mut self) {
        // The divider is written once it is let go, not on every move.
        if self.splitting {
            return;
        }
        // Off: the histories live for this run only, and what was saved is wiped.
        let keep = |v: &Vec<String>| {
            if self.config.keep_history {
                v.clone()
            } else {
                Vec::new()
            }
        };
        let state = State {
            panes: [0, 1].map(|s| PaneState {
                // One locked with dir changes allowed opens in its dir next time.
                tabs: self.panes[s]
                    .items()
                    .iter()
                    .map(|t| {
                        t.home
                            .clone()
                            .unwrap_or_else(|| t.panel.cwd().to_path_buf())
                    })
                    .collect(),
                roam: self.panes[s]
                    .items()
                    .iter()
                    .map(|t| t.home.is_some())
                    .collect(),
                active: self.panes[s].active_index(),
                locked: self.panes[s].items().iter().map(|t| t.locked).collect(),
                brief: self.panes[s].items().iter().map(|t| t.brief).collect(),
                thumbs: self.panes[s].items().iter().map(|t| t.thumbs).collect(),
                sort: (self.panes[s].items().iter())
                    .map(|t| t.panel.sort())
                    .collect(),
                names: (self.panes[s].items().iter())
                    .map(|t| t.name.clone().unwrap_or_default())
                    .collect(),
                history: if self.config.keep_history {
                    self.panes[s]
                        .items()
                        .iter()
                        .map(|t| t.history.clone())
                        .collect()
                } else {
                    Vec::new()
                },
            }),
            active: self.active,
            find: self.find.clone(),
            sync: self.sync.clone(),
            split: self.split,
            wide: self.wide,
            vertical: self.vertical,
            side_tree: self.side_tree.is_some(),
            commands: keep(&self.commands),
            masks: keep(&self.masks),
            other_cmds: keep(&self.other_cmds),
        };
        if state == self.saved {
            return;
        }
        if let Some(h) = &self.state_handler
            && let Err(e) = state.write_entry(h)
        {
            log::warn!("state: {e}");
        }
        self.saved = state;
    }

    /// Parent window of context menu popups.
    pub fn window_id(&self) -> cosmic::iced::window::Id {
        self.core
            .main_window_id()
            .unwrap_or(cosmic::iced::window::Id::RESERVED)
    }

    /// The file list's font and size.
    pub fn panel_font(&self) -> dialogs::ListFont {
        (self.face, self.config.panel_size())
    }

    /// File row height of the current skin and text size; every scroll computation uses it.
    pub fn row_h(&self) -> f32 {
        self.config.row_h()
    }

    pub(super) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A path inside a nested archive's temp copy as where that archive lies (`outer.zip/inner.zip/…`).
    pub fn shown(&self, p: &Path) -> PathBuf {
        for (tmp, back) in &self.nested {
            if let Ok(rest) = p.strip_prefix(tmp) {
                let name = tmp.file_name().unwrap_or_default();
                return self.shown(&back.join(name).join(rest));
            }
        }
        p.to_path_buf()
    }

    /// `shown`'s way back: a path through a nested archive (a click on the path line, `cd`) to its
    /// temp copy; the deepest nesting that matches.
    pub(super) fn real(&self, p: &Path) -> PathBuf {
        (self.nested.keys())
            .filter_map(|tmp| {
                let root = self.shown(tmp);
                let rest = p.strip_prefix(&root).ok()?;
                Some((root.components().count(), tmp.join(rest)))
            })
            .max_by_key(|(n, _)| *n)
            .map_or_else(|| p.to_path_buf(), |(_, real)| real)
    }

    /// The pane's path line becomes a field with its path, the cursor at its end (TC `cm_EditPath`;
    /// not selected: the selection's colour is the active line's, the path vanished in it).
    pub(super) fn edit_path(&mut self, side: usize) -> Task<Message> {
        // Not under the viewer in the panels' place: no path line there.
        if self.busy() || self.viewer() {
            return Task::none();
        }
        self.search = None;
        self.active = side;
        let shown = self.shown(self.panes[side].active().panel.cwd());
        self.path_edit = Some((side, shown.to_string_lossy().into_owned()));
        Task::batch([
            widget::text_input::focus(self.path_id.clone()),
            widget::text_input::move_cursor_to_end(self.path_id.clone()),
        ])
    }

    /// Go to `path` in the active tab (leaves search results).
    pub(super) fn load(
        &mut self,
        side: usize,
        path: PathBuf,
        focus: Option<String>,
    ) -> Task<Message> {
        let path = self.real(&path);
        // Where both are going, so quick steps on a slow fs are not lost (as `Action::Parent`).
        let from = self.panes[side].active().target();
        let other = 1 - side;
        let o = self.panes[other].active();
        // A locked tab there would open a new tab at every step: it stays (as `tree_follow`).
        let along = (self.sync_dirs && side == self.active && !o.locked)
            .then(|| {
                // Still reading: its rows are the old dir's.
                let has = |n: &std::ffi::OsStr| o.pending.is_none() && o.panel.has_dir(n);
                panel::sync_step(&from, &path, &o.target(), has)
            })
            .flatten()
            // Up out of an archive opened from inside another: that is a temp dir, it stays.
            .filter(|(to, _)| {
                (self.nested.keys()).all(|k| !o.target().starts_with(k) || to.starts_with(k))
            });
        let along = match along {
            Some((to, focus)) => self.load(other, to, focus),
            None => Task::none(),
        };
        // A locked tab keeps its dir: going elsewhere happens in a new tab next to it (TC).
        if path != self.panes[side].active().panel.cwd() {
            self.leave_locked(side);
        }
        let t = self.panes[side].active_mut();
        t.results = None;
        t.stop_branch(); // a branch read for the dir left (or re-entered) is not wanted
        let volumes = self.refresh_volumes(false);
        Task::batch([self.reload(side, path, focus), volumes, along])
    }

    /// Ask gio for unmounted volumes (`force`: now; else not within 3 s of the last time).
    pub(super) fn refresh_volumes(&mut self, force: bool) -> Task<Message> {
        let now = std::time::Instant::now();
        let recent = self
            .volumes_at
            .is_some_and(|t| now - t < std::time::Duration::from_secs(3));
        if recent && !force {
            return Task::none();
        }
        self.volumes_at = Some(now);
        blocking(mount::list, |r| Message::AllVolumes(r.unwrap_or_default()))
    }

    /// The columns' widths on screen: as kept (for the default text size) grown or shrunk with
    /// the panel's text, so a bigger font does not cut the dates (162).
    pub fn col_widths(&self) -> [f32; 4] {
        self.cols.map(|w| w * self.config.text_scale())
    }

    /// A tab locked with dir changes allowed that is no longer the shown one is read in its dir
    /// again (TC: back to the locked dir on a tab change).
    pub(super) fn roam_back(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for side in 0..2 {
            let now = self.panes[side].active().id;
            let was = std::mem::replace(&mut self.shown_tabs[side], now);
            let items = self.panes[side].items();
            let away = (items.iter().position(|t| t.id == was && was != now))
                .and_then(|i| Some((i, items[i].home.clone()?)))
                .filter(|(i, home)| items[*i].target() != *home);
            if let Some((i, home)) = away {
                tasks.push(self.load_tab(side, i, home, None));
            }
        }
        Task::batch(tasks)
    }

    /// A locked tab keeps its dir: what would move it happens in an unlocked copy opened next to it.
    pub(super) fn leave_locked(&mut self, side: usize) {
        if self.panes[side].active().locked {
            let id = self.next_id();
            let copy = self.panes[side].active().duplicate(id);
            self.panes[side].open_after(copy);
        }
    }

    /// Read the active tab's dir again (search results stay: they are re-checked).
    /// Ctrl+F8 on, off, and the keys of a tab showing the tree. `None`: not a tree (the action goes
    /// on as usual). File operations do nothing there: the files under it are not on screen.
    pub(super) fn tree_act(&mut self, side: usize, action: Action) -> Option<Task<Message>> {
        if action == Action::TreePanel {
            let t = self.panes[side].active_mut();
            if let Some(pt) = t.tree.take() {
                t.offset = pt.files_offset; // the file list where it was
                return Some(Task::batch([
                    self.restore_scroll(side),
                    self.reveal_active(side),
                ]));
            }
            return Some(self.tree_open(side));
        }
        let hidden = self.panes[side].active().panel.show_hidden();
        let tab = self.panes[side].active().id;
        let pt = self.panes[side].active_mut().tree.as_mut()?;
        let (id, tree) = (pt.id, &mut pt.tree);
        const PAGE: isize = 10;
        match action {
            Action::Up => tree.move_cursor(-1),
            Action::Down => tree.move_cursor(1),
            Action::PageUp => tree.move_cursor(-PAGE),
            Action::PageDown => tree.move_cursor(PAGE),
            Action::Home => tree.move_cursor(-isize::MAX / 2),
            Action::End => tree.move_cursor(isize::MAX / 2),
            Action::Right => {
                if let Some(dir) = tree.step_in() {
                    return Some(tree_read(id, vec![dir], hidden, None));
                }
            }
            Action::Left | Action::Parent => tree.step_out(),
            Action::QuickSearch(c) => {
                let names = tree.rows.iter().map(|n| n.name.as_str());
                if let Some(i) = shagoff_core::quicksearch::next_with(names, tree.cursor, c) {
                    tree.cursor = i;
                }
            }
            // The files of the dir chosen: in the other panel already.
            Action::Enter => {
                self.active = 1 - side;
                return Some(Task::none());
            }
            // About the files under the tree, not on screen: nothing. The rest (tabs, menus,
            // views, the terminal…) as usual.
            a if on_files(a) => return Some(Task::none()),
            _ => return None,
        }
        Some(Task::batch([
            self.reveal(side, tab),
            self.tree_follow(side),
        ]))
    }

    /// Ctrl+F8 on (or the tab's dir changed under it): the tree from `/`, opened down to the
    /// tab's dir (inside an archive: the archive's own; a nested one: where the outer lies).
    pub(super) fn tree_open(&mut self, side: usize) -> Task<Message> {
        use shagoff_core::tree::Tree;
        let hidden = self.panes[side].active().panel.show_hidden();
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let cwd = self.shown(&cwd);
        let here = match archive::split_path(&cwd) {
            Some((a, _)) => a.parent().map_or(cwd.clone(), Path::to_path_buf),
            None => cwd,
        };
        let root = PathBuf::from("/");
        let id = self.next_id();
        let t = self.panes[side].active_mut();
        let files_offset = t.tree.take().map_or(t.offset, |pt| pt.files_offset);
        t.tree = Some(PanelTree {
            id,
            tree: Tree::new(&root),
            files_offset,
        });
        t.offset = 0.0;
        let dirs = shagoff_core::tree::chain(&root, &here);
        Task::batch([
            self.restore_scroll(side),
            tree_read(id, dirs, hidden, Some(here)),
        ])
    }

    /// Ctrl+Shift+F8: the side tree put on the active panel's dir when that has changed: its row if
    /// shown, else the dirs down to it read.
    pub(super) fn side_tree_sync(&mut self) -> Task<Message> {
        if self.side_tree.is_none() {
            return Task::none();
        }
        let hidden = self.panes[self.active].active().panel.show_hidden();
        let cwd = self.panes[self.active].active().target();
        let cwd = self.shown(&cwd);
        let here = match archive::split_path(&cwd) {
            Some((a, _)) => a.parent().map_or(cwd.clone(), Path::to_path_buf),
            None => cwd,
        };
        let id = self.next_id();
        let st = self.side_tree.as_mut().expect("checked above");
        if st.synced.as_deref() == Some(here.as_path()) {
            return Task::none();
        }
        st.synced = Some(here.clone());
        if st.tree.rows.iter().any(|n| n.path == here) {
            st.tree.select(&here);
            return self.side_tree_scroll();
        }
        st.id = id;
        let root = PathBuf::from("/");
        if st.tree.rows.len() <= 1 {
            st.tree = shagoff_core::tree::Tree::new(&root);
        }
        let dirs = shagoff_core::tree::chain(&root, &here);
        tree_read(id, dirs, hidden, Some(here))
    }

    /// The side tree scrolled so its cursor row is in view (a few rows of what is above it).
    pub(super) fn side_tree_scroll(&self) -> Task<Message> {
        let Some(st) = &self.side_tree else {
            return Task::none();
        };
        let y = (st.tree.cursor.saturating_sub(3)) as f32 * self.row_h();
        scrollable::scroll_to(
            self.side_scroll.clone(),
            AbsoluteOffset {
                x: None,
                y: Some(y),
            },
        )
    }

    /// `reveal` for the active tab of `side`.
    pub(super) fn reveal_active(&mut self, side: usize) -> Task<Message> {
        let tab = self.panes[side].active().id;
        self.reveal(side, tab)
    }

    /// The other panel to the dir under tab `side`'s tree cursor (TC), unless it is there.
    pub(super) fn tree_follow(&mut self, side: usize) -> Task<Message> {
        let Some(dir) = (self.panes[side].active().tree.as_ref())
            .and_then(|pt| pt.tree.current())
            .map(|n| n.path.clone())
        else {
            return Task::none();
        };
        // A locked tab there would open a new tab at every step: it stays.
        let other = self.panes[1 - side].active();
        if other.target() == dir || other.locked {
            return Task::none();
        }
        self.load(1 - side, dir, None)
    }

    /// The panel holding tab `tab` now (tabs move: Ctrl+U, a drag), `None` once it is closed.
    pub(super) fn side_of(&self, tab: u64) -> Option<usize> {
        (0..2).find(|&s| self.panes[s].items().iter().any(|t| t.id == tab))
    }

    /// Scan `path` for the active tab of `side` in the background; the result lands in `Message::Listed`.
    pub(super) fn reload(
        &mut self,
        side: usize,
        path: PathBuf,
        focus: Option<String>,
    ) -> Task<Message> {
        let i = self.panes[side].active_index();
        self.load_tab(side, i, path, focus)
    }

    /// Scan `path` for tab `i` of `side` in the background.
    pub(super) fn load_tab(
        &mut self,
        side: usize,
        i: usize,
        path: PathBuf,
        focus: Option<String>,
    ) -> Task<Message> {
        let generation = self.next_id();
        let t = &mut self.panes[side].items_mut()[i];
        t.pending = Some((generation, path.clone()));
        t.pending_at = Instant::now();
        // Going anywhere else leaves the search results.
        if t.results.as_ref().is_some_and(|(root, _)| *root != path) {
            t.results = None;
        }
        let results = t.results.as_ref().map(|(_, r)| r.clone());
        // Ctrl+B's second step: Esc stops it as it stops the first.
        let stop = match t.branching {
            Some(_) => t.branch_stop.clone(),
            None => Arc::default(),
        };
        let tab = t.id;
        let (show_hidden, sort) = (t.panel.show_hidden(), t.panel.sort());
        // The dir shown read again (restored at startup, a watcher event, Ctrl+R) and gone: up to
        // one that is there, as TC. Found by the background read — never a `stat` on the UI
        // thread, a dead mount would hang it. A step elsewhere into a gone dir stays an error.
        let resolve = (path == t.panel.cwd()).then(|| self.home.clone());
        Task::perform(
            async move {
                let (result, space, path) = tokio::task::spawn_blocking(move || {
                    let p = match resolve {
                        Some(home) => session::existing_dir(&path, &home),
                        None => path,
                    };
                    let mut result = read_listing(&p, show_hidden, results.as_deref(), &stop);
                    // Sorted here, not on the UI thread; the panel only checks the order.
                    // ponytail: counted dir sizes are not known here; a Size sort with them is
                    // redone by the panel.
                    if let Ok(entries) = &mut result {
                        shagoff_core::sort::sort_entries(entries, sort, &HashMap::new());
                    }
                    let space = drives::space(&p);
                    (result, space, p)
                })
                .await
                .unwrap_or_else(|e| (Err(e.to_string()), None, PathBuf::new()));
                Message::Listed {
                    tab,
                    generation,
                    path,
                    focus,
                    result,
                    space,
                }
            },
            cosmic::Action::App,
        )
    }

    /// A click in pane `side`: as a key there (and Tab away from the other) — the error shown
    /// in the pane left and in this one goes, quick search closes.
    pub(super) fn click_pane(&mut self, side: usize) {
        self.search = None;
        // The terminal does not see a click its rows took first: the keys come back to the panels.
        self.term_blur = self.term.is_some();
        for s in [self.active, side] {
            self.panes[s].active_mut().error = None;
        }
        self.active = side;
    }
}
