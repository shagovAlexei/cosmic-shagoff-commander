//! Thumbnails asked for, quick search / filter, status messages, the side drawer.

use super::*;

impl App {
    /// Thumbnails view: start making the pictures on screen that are not made yet, a few at a
    /// time (each one done asks for the next through `update`).
    pub(super) fn want_thumbs(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let cell_h = crate::thumbs::cell_h(self.row_h());
        let covered = self.viewer_shown();
        for side in 0..2 {
            let t = self.panes[side].active();
            let hidden = covered.is_some_and(|q| q.is_none_or(|s| s == side));
            if !t.thumbs || hidden || archive::within(t.panel.cwd()) {
                continue;
            }
            let entries = t.panel.entries();
            let range =
                viewport::thumb_range(entries.len(), t.thumb_cols(), cell_h, t.offset, t.height);
            for e in &entries[range] {
                if self.thumbs_busy >= crate::thumbs::BUSY_MAX {
                    break;
                }
                if e.is_dir() {
                    continue;
                }
                let path = t.panel.cwd().join(&e.os_name);
                if self.thumbs.get(&path).is_some_and(|(m, _)| *m == e.mtime)
                    || !crate::thumbs::wanted(&e.name, e.size)
                {
                    continue;
                }
                // ponytail: the whole cache goes at once; an LRU if big picture dirs thrash it.
                if self.thumbs.len() >= crate::thumbs::CACHE_MAX {
                    self.thumbs.clear();
                }
                self.thumbs.insert(path.clone(), (e.mtime, None));
                self.thumbs_busy += 1;
                let mtime = e.mtime;
                let p = path.clone();
                tasks.push(Task::perform(
                    async move {
                        (tokio::task::spawn_blocking(move || crate::thumbs::make(&p)).await)
                            .ok()
                            .flatten()
                    },
                    move |pic| cosmic::Action::App(Message::Thumb(path, mtime, pic)),
                ));
            }
        }
        Task::batch(tasks)
    }

    /// Keep tab `tab`'s cursor row fully visible. An inactive tab only gets its offset updated;
    /// `restore_scroll` applies it when that tab is shown.
    pub(super) fn reveal(&mut self, side: usize, tab: u64) -> Task<Message> {
        let is_active = self.panes[side].active().id == tab;
        let row_h = self.row_h();
        let Some(t) = self.panes[side]
            .items_mut()
            .iter_mut()
            .find(|t| t.id == tab)
        else {
            return Task::none();
        };
        if t.brief && t.tree.is_none() {
            let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
            let len = t.panel.entries().len();
            t.col = viewport::brief_first_col(len, t.panel.cursor(), rows, cols, t.col);
            return Task::none();
        }
        let (row, row_h) = match (&t.tree, t.thumbs) {
            (Some(pt), _) => (pt.tree.cursor, row_h),
            (None, true) => (
                t.panel.cursor() / t.thumb_cols(),
                crate::thumbs::cell_h(row_h),
            ),
            (None, false) => (t.panel.cursor(), row_h),
        };
        match viewport::scroll_to_cursor(row, row_h, t.offset, t.height) {
            Some(y) => {
                t.offset = y;
                if is_active {
                    self.restore_scroll(side)
                } else {
                    Task::none()
                }
            }
            None => Task::none(),
        }
    }

    /// Alt+letter: append to an open search (or start one) if the text still matches something.
    pub(super) fn quick_search(&mut self, side: usize, c: char) -> Task<Message> {
        let mut text = match &self.search {
            Some(s) if s.side == side && !s.filter => s.text.clone(),
            _ => String::new(),
        };
        text.push(c);
        let Some(i) = self.panes[side].active().panel.find(&text, 0, true) else {
            return Task::none(); // TC: a letter that finds nothing is not taken
        };
        self.search = Some(Search {
            side,
            text,
            filter: false,
        });
        self.active = side;
        let t = self.panes[side].active_mut();
        t.panel.set_cursor(i);
        let tab = t.id;
        Task::batch([
            widget::text_input::focus(self.input_id.clone()),
            self.reveal(side, tab),
        ])
    }

    pub(super) fn quick_filter(&mut self, side: usize) -> Task<Message> {
        let text = self.panes[side]
            .active()
            .panel
            .filter()
            .unwrap_or_default()
            .to_string();
        self.search = Some(Search {
            side,
            text,
            filter: true,
        });
        self.active = side;
        widget::text_input::focus(self.input_id.clone())
    }

    pub(super) fn say(&mut self, kind: StatusKind, text: String) {
        self.status = Some(Status { kind, text });
    }

    /// A background job finished fine: drop its "working…", not a newer message.
    // ponytail: two mounts at once share one Busy slot; the first result clears it.
    pub(super) fn clear_busy(&mut self) {
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.kind == StatusKind::Busy)
        {
            self.status = None;
        }
    }

    /// The status bar text (tests).
    #[cfg(test)]
    pub(super) fn msg(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.text.as_str())
    }

    pub(super) fn set_drawer(&mut self, d: Option<Drawer>) {
        self.core.window.show_context = d.is_some();
        // A new look at About asks again: an old answer may no longer hold.
        if d == Some(Drawer::About) {
            self.update_check = None;
        }
        self.drawer = d;
    }

    /// F1 / About / Ctrl+,: open that drawer, or close it if it is the one open.
    pub(super) fn toggle_drawer(&mut self, action: Action) {
        let open = match action {
            Action::Help => Some(Drawer::Help),
            Action::About => Some(Drawer::About),
            Action::Donate => Some(Drawer::Donate),
            _ => None,
        };
        let same = match (&self.drawer, &open) {
            (Some(cur), Some(d)) => std::mem::discriminant(cur) == std::mem::discriminant(d),
            (Some(cur), None) => matches!(cur, Drawer::Settings(_)),
            (None, _) => false,
        };
        if same {
            return self.set_drawer(None);
        }
        // Settings: the form (and the installed fonts) only when they open.
        let d = open.unwrap_or_else(|| {
            Drawer::Settings(SettingsForm {
                viewer: self.config.viewer.join(" "),
                editor: self.config.editor.join(" "),
                home: self
                    .config
                    .home_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                ignore: self.config.ignore_list.clone(),
                fonts: drawer::font_families(),
                font_filter: String::new(),
            })
        });
        self.set_drawer(Some(d));
    }
}
