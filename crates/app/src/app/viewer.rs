//! F3 viewer and Ctrl+Q quick view.

use super::*;

impl App {
    /// Esc in the viewer: the search field first, then the selection, then the viewer.
    pub(super) fn lister_escape(&mut self) -> Task<Message> {
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        if l.searching {
            l.searching = false;
        } else if l.sel.is_some() {
            l.sel = None;
        } else {
            return self.handle(Message::ListerClose);
        }
        Task::none()
    }

    /// The full F3 viewer is up (not the Ctrl+Q one) and takes the keys: in place of the panels,
    /// or its own window focused (no window known to be focused: as focused, so it is never left
    /// without keys).
    pub(super) fn viewer(&self) -> bool {
        self.lister.as_ref().is_some_and(|l| {
            l.quick.is_none()
                && l.window
                    .is_none_or(|w| self.core.focused_window().is_none_or(|f| f == w))
        })
    }

    /// Ctrl+Q: show what is under the active pane's cursor, if that changed. Run after every
    /// message, so any way the cursor moves (keys, mouse, quick search, a rescan) is followed.
    pub(super) fn quick_follow(&mut self) -> Task<Message> {
        let side = self.active;
        let panel = &self.panes[side].active().panel;
        let entry = (panel.current()).filter(|e| e.name != PARENT);
        let key = entry.map_or_else(
            || (panel.cwd().to_path_buf(), 0, std::time::UNIX_EPOCH),
            |e| (panel.cwd().join(&e.os_name), e.size, e.mtime),
        );
        let (name, dir) = entry.map_or((String::new(), None), |e| {
            (e.name.clone(), Some(e.is_dir()))
        });
        match self.lister.as_deref() {
            // Only the Ctrl+Q one; the F3 viewer stays on its file.
            Some(l)
                if l.quick
                    .as_ref()
                    .is_some_and(|q| *q != key || l.side != side) => {}
            Some(l) if l.quick.is_some() => return Task::none(),
            // Closed (or never open): a dir count left running has no one to show it to.
            _ => {
                self.quick_stop.store(true, Ordering::Relaxed);
                return Task::none();
            }
        }
        let id = self.next_id();
        self.quick_latest.store(id, Ordering::Relaxed);
        let latest = self.quick_latest.clone();
        // A dir still being counted for the last entry: not any more.
        let stop = std::mem::take(&mut self.quick_stop);
        stop.store(true, Ordering::Relaxed);
        let stop = self.quick_stop.clone();
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        let (file, size) = (key.0.clone(), key.1);
        l.reopen(name.clone(), id);
        l.quick = Some(key);
        l.side = side;
        // Inside an archive? As F3 decides (a real dir named `x.zip` is none).
        let in_archive = archive::split_path(self.panes[side].active().panel.cwd());
        if in_archive.is_none() {
            l.file = Some(file.clone());
        }
        if dir.is_none() {
            l.set_loaded(Arc::new(Err(fl!("quick-none"))));
            return Task::none();
        }
        // The viewer reads 32 MB at most; a bigger entry would be unpacked whole for that.
        if in_archive.is_some() && size > shagoff_core::lister::LIMIT {
            let limit = (shagoff_core::lister::LIMIT >> 20).to_string();
            l.set_loaded(Arc::new(Err(fl!("quick-too-big", limit = limit))));
            return Task::none();
        }
        // An archive opens as a folder here: shown like one, what it holds (F3 shows its bytes).
        let whole = in_archive.is_none()
            && dir == Some(false)
            && Format::detect(&name).is_some_and(Format::is_tree);
        // TC: a dir shows what is in it, counted in the background.
        if dir == Some(true) || whole {
            l.set_loaded(Arc::new(Err(fl!("quick-counting"))));
            let entry = file.file_name().map(PathBuf::from).unwrap_or_default();
            // (archive, dir inside it) when counted from an archive's index.
            let indexed = match in_archive {
                _ if whole => Some((file.clone(), PathBuf::new())),
                Some((arc, inner)) => Some((arc, inner.join(entry))),
                None => None,
            };
            let key = if whole { "archive" } else { "dir" };
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || match indexed {
                        // From the archive's index: nothing is unpacked.
                        Some((arc, inner)) => {
                            // An unreadable index (archive gone, cut): say so, not a blank.
                            archive::usage(&arc, &inner).map_err(|_| ())
                        }
                        None => {
                            // A link to a dir: what it points to (`usage` does not follow links).
                            let root = std::fs::canonicalize(&file).unwrap_or(file);
                            // Stopped: dropped anyway (the cursor moved on, its id is stale).
                            shagoff_core::props::usage(&[root], &stop).ok_or(())
                        }
                    })
                    .await
                    .unwrap_or(Err(()))
                },
                move |u| {
                    let text = match u {
                        Ok(u) => {
                            let (files, dirs) = (u.files, u.dirs.saturating_sub(1));
                            let size = crate::view::human(u.bytes);
                            if key == "archive" {
                                fl!("quick-archive", files = files, dirs = dirs, size = size)
                            } else {
                                fl!("quick-dir", files = files, dirs = dirs, size = size)
                            }
                        }
                        Err(()) => fl!("quick-none"),
                    };
                    cosmic::Action::App(Message::ListerLoaded(id, Arc::new(Err(text))))
                },
            );
        }
        let read = Task::perform(
            async move {
                // Held ↓ passes many files: read only the one the cursor stops on.
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                if latest.load(Ordering::Relaxed) != id {
                    return Err(String::new()); // dropped: its id is stale
                }
                tokio::task::spawn_blocking(move || match in_archive {
                    // Inside an archive: taken out into a temp dir, read, the copy dropped.
                    Some((arc, inner)) => {
                        let tmp = archive::fresh_temp_dir(&archive::temp_root())
                            .map_err(|e| crate::why::text(&e))?;
                        let entry = file.file_name().map(PathBuf::from).unwrap_or_default();
                        let r = match archive::extract_one(&arc, &inner, &entry, &tmp, &stop) {
                            Some(f) => lister::Loaded::read(&f, &name),
                            None => Err(fl!("quick-none")),
                        };
                        let _ = std::fs::remove_dir_all(&tmp);
                        r
                    }
                    None => lister::Loaded::read(&file, &name),
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerLoaded(id, Arc::new(r))),
        );
        Task::batch([read, lister_scroll(l, 0.0, 0.0)])
    }

    /// F3 with the built-in viewer: the file under the cursor (extracted first inside an archive).
    pub(super) fn view_current(&mut self, side: usize) -> Task<Message> {
        let panel = &self.panes[side].active().panel;
        let cwd = panel.cwd().to_path_buf();
        let Some(e) = panel.current().filter(|e| !e.is_dir() && e.name != PARENT) else {
            return Task::none();
        };
        let (os_name, name) = (e.os_name.clone(), e.name.clone());
        let (dir, own) = located(&cwd, &os_name);
        if let Some((arc, inner)) = archive::split_path(&dir) {
            return self.open_from_archive(side, arc, inner, own, How::Lister);
        }
        let file = cwd.join(&os_name);
        if let Some(why) = unopenable(&file, &name) {
            self.say(StatusKind::Error, why);
            return Task::none();
        }
        self.open_lister(side, file, name)
    }

    pub(super) fn open_lister(
        &mut self,
        side: usize,
        file: PathBuf,
        name: String,
    ) -> Task<Message> {
        let id = self.next_id();
        let mut window = Task::none();
        // Ctrl+Q's viewer: the full one replaces it (in its own window, if so set).
        if self.lister.as_ref().is_some_and(|l| l.quick.is_some()) {
            self.lister = None;
        }
        match &mut self.lister {
            Some(l) => {
                l.reopen(name.clone(), id);
                l.side = side; // N / P walk the pane F3 was pressed in
                if let Some(w) = l.window {
                    window = Task::batch([
                        cosmic::ApplicationExt::set_window_title(self, name.clone(), w),
                        cosmic::iced::window::gain_focus(w),
                    ]);
                }
            }
            None => {
                let mut l = Lister::new(side, name.clone(), id, self.config.lister_size());
                // TC: the Lister is a window of its own.
                if self.config.lister_window {
                    let (w, open) = cosmic::iced::window::open(cosmic::iced::window::Settings {
                        size: cosmic::iced::Size::new(900.0, 650.0),
                        min_size: Some(cosmic::iced::Size::new(360.0, 240.0)),
                        decorations: false,
                        exit_on_close_request: false,
                        ..Default::default()
                    });
                    l.window = Some(w);
                    window = Task::batch([
                        open.discard(),
                        cosmic::ApplicationExt::set_window_title(self, name.clone(), w),
                    ]);
                }
                self.lister = Some(Box::new(l));
            }
        }
        self.search = None;
        if let Some(l) = self.lister.as_deref_mut() {
            l.file = Some(file.clone());
        }
        let read = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lister::Loaded::read(&file, &name))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerLoaded(id, Arc::new(r))),
        );
        let top = self
            .lister
            .as_deref_mut()
            .map_or(Task::none(), |l| lister_scroll(l, 0.0, 0.0));
        Task::batch([read, top, window])
    }

    /// Keys while the viewer is open; `None`: not the viewer's (help, settings…), act as usual.
    pub(super) fn lister_action(&mut self, action: Action) -> Option<Task<Message>> {
        if matches!(action, Action::Left | Action::Right) {
            self.lister.as_ref()?;
            return Some(self.lister_scroll_x(action == Action::Right));
        }
        let l = self.lister.as_deref_mut()?;
        Some(match action {
            Action::Help | Action::About | Action::Settings | Action::Donate | Action::F12 => {
                return None;
            }
            // Num+ / Num-: bigger / smaller text, as + / - (TC zooms with them).
            Action::SelectGroup => return Some(self.lister_zoom(true)),
            Action::UnselectGroup => return Some(self.lister_zoom(false)),
            Action::View => self.lister_find(true, true),
            Action::Mkdir => self.lister_search(),
            Action::SelectAll => {
                l.select_all();
                Task::none()
            }
            Action::ClipCopy => match l.selected_text() {
                Some(t) => {
                    let n = t.chars().count();
                    self.say(StatusKind::Info, fl!("lister-copied", n = n));
                    cosmic::iced::clipboard::write(t)
                }
                None => Task::none(),
            },
            // Home / End of a big file: its first / last window.
            Action::Home if l.window_for(Slide::Home).is_some() => self.lister_slide(Slide::Home),
            Action::End if l.window_for(Slide::End).is_some() => self.lister_slide(Slide::End),
            a => match l.scroll_y(a) {
                Some(y) => {
                    let scroll = lister_scroll(l, l.offset.0, y);
                    match l.edge() {
                        Some(s) => Task::batch([scroll, self.lister_slide(s)]),
                        None => scroll,
                    }
                }
                None => Task::none(),
            },
        })
    }

    /// Read another window of the viewer's big file (`Message::ListerPart`).
    pub(super) fn lister_slide(&mut self, slide: Slide) -> Task<Message> {
        let Some(l) = self.lister.as_deref_mut() else {
            return Task::none();
        };
        let (Some(w), Some(file)) = (l.window_for(slide), l.file.clone()) else {
            return Task::none();
        };
        // Home / End may come while another is read: its reply is dropped (a search's too).
        let searching = matches!(l.slide, Some(Slide::Find(_)));
        l.slide = Some(slide);
        let id = l.id;
        if searching {
            self.clear_busy();
        }
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lister::Loaded::window(&file, w))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| cosmic::Action::App(Message::ListerPart(id, slide, Arc::new(r))),
        )
    }

    /// The viewer's text one size up or down: the same setting as in Settings, saved.
    pub(super) fn lister_zoom(&mut self, up: bool) -> Task<Message> {
        // A picture: its zoom (TC), never the text size; still reading: not known which yet.
        if let Some(l) = self.lister.as_deref_mut()
            && (l.loaded.is_none() || l.mode == shagoff_core::lister::Mode::Image)
        {
            return match l.zoom_image(up) {
                Some((x, y)) => lister_scroll(l, x, y),
                None => Task::none(),
            };
        }
        let size = config::size_step(self.config.lister_size(), up);
        if size == self.config.lister_size() {
            return Task::none();
        }
        self.set(crate::drawer::Setting::ListerSize(size))
    }

    pub(super) fn lister_search(&mut self) -> Task<Message> {
        let Some(l) = &mut self.lister else {
            return Task::none();
        };
        l.searching = true;
        widget::text_input::focus(l.input.clone())
    }

    /// Enter in the field (`again` false) or F3 / Shift+F3; no text yet → open the field.
    pub(super) fn lister_find(&mut self, forward: bool, again: bool) -> Task<Message> {
        let Some(l) = &mut self.lister else {
            return Task::none();
        };
        if l.query.is_empty() {
            return self.lister_search();
        }
        match l.find(forward, again) {
            Some(y) => lister_scroll(l, 0.0, y),
            // Not in this window of a big file: on through the next ones.
            None if let Some(s) = l.search_on(forward) => {
                let q = l.query.clone();
                self.say(StatusKind::Busy, fl!("lister-searching", query = q));
                self.lister_slide(s)
            }
            None => {
                let q = l.query.clone();
                self.say(StatusKind::Info, fl!("lister-not-found", query = q));
                Task::none()
            }
        }
    }

    pub(super) fn lister_scroll_x(&mut self, right: bool) -> Task<Message> {
        self.lister.as_deref_mut().map_or(Task::none(), |l| {
            let (x, y) = (l.scroll_x(right), l.offset.1);
            lister_scroll(l, x, y)
        })
    }

    /// N / P: the next / previous file of the pane (dirs skipped), cursor moved there too.
    pub(super) fn lister_step(&mut self, forward: bool) -> Task<Message> {
        let Some(side) = self.lister.as_ref().map(|l| l.side) else {
            return Task::none();
        };
        let t = self.panes[side].active_mut();
        let entries = t.panel.entries();
        let file = |i: &usize| !entries[*i].is_dir() && entries[*i].name != PARENT;
        let cur = t.panel.cursor();
        let next = if forward {
            (cur + 1..entries.len()).find(file)
        } else {
            (0..cur).rev().find(file)
        };
        let Some(i) = next else {
            return Task::none();
        };
        t.panel.set_cursor(i);
        let tab = t.id;
        let shown = self.lister.as_ref().map(|l| l.id);
        let view = self.view_current(side);
        if self.job.is_none() && self.lister.as_ref().map(|l| l.id) == shown {
            // Not opened (broken link): the cursor stays with the file on screen.
            self.panes[side].active_mut().panel.set_cursor(cur);
            return view;
        }
        Task::batch([self.reveal(side, tab), view])
    }
}
