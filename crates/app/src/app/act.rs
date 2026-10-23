//! Keys and menu items → `App::act`, the `match` over `Action`.

use super::*;

impl App {
    pub(super) fn act(&mut self, side: usize, action: Action) -> Task<Message> {
        // An error stays in the status line until the next action in that pane.
        self.panes[side].active_mut().error = None;
        // A message lasts until the next action; "working…" until its result.
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.kind != StatusKind::Busy)
        {
            self.status = None;
        }
        if let Some(task) = self.tree_act(side, action) {
            return task;
        }
        // TC: Shift+F6 on `..` with nothing marked renames the path, i.e. edits it.
        let p = &self.panes[side].active().panel;
        let on_parent = p.current().is_some_and(|e| e.name == PARENT) && p.targets().is_empty();
        if action == Action::EditPath || (action == Action::Rename && on_parent) {
            return self.edit_path(side);
        }
        // Names there are paths: Ctrl+M would move files between dirs, Shift+F2 compares by name.
        if matches!(
            action,
            Action::MultiRename
                | Action::CompareLists
                | Action::CompareListsSubdirs
                | Action::CompareListsDiffOnly
        ) && (0..2).any(|s| self.panes[s].active().results.is_some())
        {
            self.say(StatusKind::Error, fl!("results-unsupported"));
            return Task::none();
        }
        if let Some(task) = self.cmd_action(side, action) {
            return task;
        }
        // Backspace / ".." in search results: back to the dir that was searched.
        let t = self.panes[side].active_mut();
        let on_parent = t.panel.current().is_some_and(|e| e.name == PARENT);
        if (action == Action::Parent || (action == Action::Enter && on_parent))
            && let Some((root, _)) = t.results.take()
        {
            return self.load(side, root, None);
        }
        match action {
            Action::QuickSearch(c) => return self.quick_search(side, c),
            Action::QuickFilter => return self.quick_filter(side),
            Action::Disconnect => return self.disconnect(side),
            Action::OpenWith => return self.open_with(side),
            Action::Help | Action::About | Action::Settings | Action::Donate => {
                self.toggle_drawer(action);
                return Task::none();
            }
            // Nothing set: the settings (left open if they are already).
            Action::F12 if self.config.f12.cmd.trim().is_empty() => {
                if !matches!(self.drawer, Some(Drawer::Settings(_))) {
                    self.toggle_drawer(Action::Settings);
                }
                return Task::none();
            }
            Action::F12 => return self.run_button(self.config.f12.clone()),
            Action::ToggleHumanSizes => {
                return self.set(Setting::HumanSizes(!self.config.human_sizes()));
            }
            Action::ToggleDividerLine => {
                return self.set(Setting::DividerLine(!self.config.divider_line));
            }
            Action::SwitchIgnoreList => return self.set(Setting::IgnoreOn(!self.config.ignore_on)),
            Action::SwitchWatchDirs => {
                return self.set(Setting::WatchDirs(!self.config.watch_dirs));
            }
            Action::SwitchDirSort => {
                return self.set(Setting::DirsByName(!self.config.dirs_by_name));
            }
            Action::SwitchTypeColors => {
                return self.set(Setting::TypeColors(!self.config.type_colors));
            }
            Action::ToggleActiveFrame => {
                return self.set(Setting::ActiveFrame(!self.config.active_frame));
            }
            Action::ToggleTerminal => {
                let on = !self.config.term_builtin;
                self.term_want_focus = on;
                let task = self.set(Setting::TermBuiltin(on));
                // Hidden while it had the keys: back to the panels.
                return match on {
                    true => task,
                    false => Task::batch([task, unfocus()]),
                };
            }
            Action::TermFull => {
                if !self.config.term_builtin {
                    return Task::none();
                }
                if self.term.is_none() {
                    self.term_ended = false;
                    self.term_start();
                }
                self.term_full = !self.term_full && self.term.is_some();
                // Over the whole height: its keys; back: the panels'.
                return match self.term_full {
                    true => cosmic::iced::runtime::widget::operation::focus(self.term_id.clone()),
                    false => unfocus(),
                };
            }
            Action::PickShell => {
                let (items, cursor) = self.shell_items();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Shells,
                    side,
                    cursor,
                    items,
                });
                return Task::none();
            }
            Action::CopySrcPath | Action::CopyTrgPath | Action::CopyDirsOfFiles => {
                let s = match action {
                    Action::CopyTrgPath => 1 - side,
                    _ => side,
                };
                let panel = &self.panes[s].active().panel;
                let mut dirs: Vec<PathBuf> = match action {
                    // In search results each file's own dir; once each, in the panel's order.
                    Action::CopyDirsOfFiles => (panel.targets().iter())
                        .filter_map(|p| p.parent().map(Path::to_path_buf))
                        .collect(),
                    _ => vec![panel.cwd().to_path_buf()],
                };
                let mut seen = std::collections::HashSet::new();
                dirs.retain(|d| seen.insert(d.clone()));
                if dirs.is_empty() {
                    return Task::none();
                }
                let dirs: Vec<PathBuf> = dirs.iter().map(|d| self.shown(d)).collect();
                let text = shagoff_core::clipboard::names_text(&dirs, true);
                self.say(
                    StatusKind::Info,
                    fl!("copied-paths", n = dirs.len(), text = text.clone()),
                );
                return cosmic::iced::clipboard::write(text);
            }
            Action::CopyDetails | Action::CopyDetailsHdr => {
                let panel = &self.panes[side].active().panel;
                let rows = panel.target_entries();
                if rows.is_empty() {
                    return Task::none();
                }
                let n = rows.len();
                let text = self.details_text(panel, &rows, action == Action::CopyDetailsHdr);
                self.say(StatusKind::Info, fl!("copied-details", n = n));
                return cosmic::iced::clipboard::write(text);
            }
            Action::CopyNames | Action::CopyPaths => {
                let paths = self.panes[side].active().panel.targets();
                if paths.is_empty() {
                    return Task::none();
                }
                let text = shagoff_core::clipboard::names_text(&paths, action == Action::CopyPaths);
                let (n, one) = (paths.len(), text.clone());
                let msg = if action == Action::CopyPaths {
                    fl!("copied-paths", n = n, text = one)
                } else {
                    fl!("copied-names", n = n, text = one)
                };
                self.say(StatusKind::Info, msg);
                return cosmic::iced::clipboard::write(text);
            }
            _ => {}
        }
        if self.read_only(side, action) {
            self.say(StatusKind::Error, fl!("archive-read-only"));
            return Task::none();
        }
        // Said before its dialog (the job would be refused anyway: `changes_nested`).
        if self.nested_change(side, action) {
            self.say(StatusKind::Error, fl!("nested-read-only"));
            return Task::none();
        }
        if action == Action::Hotlist {
            self.hot_sub.clear(); // Ctrl+D always opens at the top
        }
        if let Some(d) = self.dialog_for(side, action) {
            if matches!(d, Dialog::Sync(_)) {
                self.dialog = Some(d);
                return self.start_compare(); // TC compares right away
            }
            if matches!(d, Dialog::Diff(_)) {
                self.dialog = Some(d);
                return self.start_diff();
            }
            if matches!(d, Dialog::Props(_)) {
                self.dialog = Some(d);
                return self.start_usage();
            }
            if let Dialog::List {
                kind: ListKind::Drives,
                side: s,
                ..
            } = d
            {
                self.dialog = Some(d);
                return list_volumes(s);
            }
            let focus = matches!(
                d,
                Dialog::Mask { .. }
                    | Dialog::Input { .. }
                    | Dialog::MultiRename(_)
                    | Dialog::Pack(_)
                    | Dialog::Unpack { .. }
                    | Dialog::Find(_)
                    | Dialog::Connect { .. }
            );
            self.dialog = Some(d);
            return if focus {
                widget::text_input::focus(self.input_id.clone())
            } else {
                Task::none()
            };
        }
        let row_h = self.row_h();
        let t = self.panes[side].active_mut();
        let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
        let (brief, rows) = (t.brief, rows as isize);
        let (thumbs, across) = (t.thumbs, t.thumb_cols() as isize);
        let page = if brief {
            rows * cols as isize
        } else if thumbs {
            viewport::page_rows(crate::thumbs::cell_h(row_h), t.height) as isize * across
        } else {
            viewport::page_rows(row_h, t.height) as isize
        };
        let tab = t.id;
        let target = t.target();
        let loading = target != t.panel.cwd();
        // A double Enter must not open a row of the dir being left; a read hung on a dead mount
        // must not hold Enter for good: after a moment the rows on screen are what it opens.
        let stepping = loading && t.pending_at.elapsed() < ENTER_WAIT;
        let panel = &mut t.panel;
        match action {
            Action::SwitchPane => {
                self.active = 1 - self.active;
                return Task::none();
            }
            Action::Up if thumbs => panel.move_cursor(-across),
            Action::Down if thumbs => panel.move_cursor(across),
            Action::Left if thumbs => panel.move_cursor(-1),
            Action::Right if thumbs => panel.move_cursor(1),
            Action::Up => panel.move_cursor(-1),
            Action::Down => panel.move_cursor(1),
            Action::Left if brief => panel.move_cursor(-rows),
            Action::Right if brief => panel.move_cursor(rows),
            Action::Left | Action::Right => {}
            Action::ConfigureColors => {
                let mut edit = crate::colorrules::ColorEdit::new(self.config.colors.clone());
                edit.panel = self.config.panel.clone();
                self.dialog = Some(Dialog::Colors(Box::new(edit)));
                return Task::none();
            }
            Action::OpenTerminal => {
                self.open_terminal();
                return Task::none();
            }
            Action::PickTerminal => {
                let (items, cursor) = self.terminal_items();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Terminals,
                    side,
                    cursor,
                    items,
                });
                return Task::none();
            }
            Action::ConfigureKeys => {
                let list = self.config.hotkeys.clone();
                self.dialog = Some(Dialog::Keys(Box::new(crate::hotkeys::KeyEdit::new(list))));
                return Task::none();
            }
            Action::ConfigureStartMenu => {
                let t = crate::toolbar::ToolEdit::menu(self.config.start_menu.clone());
                self.dialog = Some(Dialog::Toolbar(Box::new(t)));
                return Task::none();
            }
            Action::UserMenu(i) => {
                return match self.config.start_menu.get(i) {
                    Some(b) => self.run_button(b.clone()),
                    None => Task::none(),
                };
            }
            Action::ConfigureToolbar => {
                let list = self.config.toolbar.clone();
                self.dialog = Some(Dialog::Toolbar(Box::new(crate::toolbar::ToolEdit::new(
                    list,
                ))));
                return Task::none();
            }
            Action::ResetColumns => {
                self.cols = Config::default().columns;
                self.save_columns();
                return Task::none();
            }
            Action::ViewBrief | Action::ViewFull | Action::ViewThumbs => {
                let t = self.panes[side].active_mut();
                t.brief = action == Action::ViewBrief;
                t.thumbs = action == Action::ViewThumbs;
                return Task::batch([self.reveal(side, tab), self.restore_scroll(side)]);
            }
            Action::PageUp => panel.move_cursor(-page),
            Action::PageDown => panel.move_cursor(page),
            Action::Home => panel.cursor_home(),
            Action::End => panel.cursor_end(),
            // 100 000 rows take ~0.1 s to sort: a big list is sorted by a background read.
            // Unsorted: read again, the rows here are in the last sort's order.
            Action::Sort(key)
                if key == shagoff_core::sort::SortKey::Unsorted
                    || panel.entries().len() > SORT_HERE_MAX =>
            {
                panel.set_sort_key(key);
                return self.reload(side, target, None);
            }
            Action::Sort(key) => panel.set_sort(key),
            // The rows on screen belong to the dir being left: entering one would undo the navigation.
            // Shift+Enter, no line typed: the file under the cursor in the terminal (a dir: Enter).
            Action::RunTerminal => {
                let cwd = panel.cwd().to_path_buf();
                let file = (panel.current())
                    .filter(|e| e.kind == shagoff_core::listing::Kind::File)
                    .map(|e| e.name.clone());
                let Some(name) = file else {
                    return self.act(side, Action::Enter);
                };
                if archive::within(&cwd) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
                    return Task::none();
                }
                let prefix = self.terminal_prefix();
                let line = cmdline::quote(&format!("./{name}"));
                if let Err(e) = spawn_in(&cmdline::argv(&line, &cwd, Some(&prefix)), &cwd) {
                    self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                }
            }
            Action::Enter if stepping => {}
            Action::Enter => {
                // ".." at a nested archive's root: back into the outer one, as Backspace.
                if panel.current().is_some_and(|e| e.name == PARENT)
                    && self.nested.contains_key(panel.cwd())
                {
                    return self.act(side, Action::Parent);
                }
                if let Some((path, focus)) = panel.enter_path() {
                    return self.load(side, path, focus);
                }
                let cwd = panel.cwd().to_path_buf();
                let current = panel.current().map(|e| (e.os_name.clone(), e.name.clone()));
                if let Some((os_name, name)) = current {
                    let (dir, own) = located(&cwd, &os_name);
                    match archive::split_path(&dir) {
                        Some((arc, inner)) => {
                            let os_name = own;
                            let how = match Format::detect(&name).is_some_and(Format::is_tree) {
                                true => How::Browse,
                                false => How::Run(Vec::new(), &["xdg-open"]),
                            };
                            return self.open_from_archive(side, arc, inner, os_name, how);
                        }
                        // Not inside one already: archives inside archives open as files.
                        None if Format::detect(&name).is_some_and(Format::is_tree) => {
                            return self.load(side, cwd.join(os_name), None);
                        }
                        None => {}
                    }
                }
                let t = self.panes[side].active_mut();
                let panel = &t.panel;
                let file = panel.current().map(|e| panel.cwd().join(&e.os_name));
                if let Some(file) = file
                    && let Err(err) = spawn_detached(&launch::command(&[], &["xdg-open"], &file))
                {
                    self.say(
                        StatusKind::Error,
                        fl!("open-failed", err = crate::why::text(&err)),
                    );
                }
            }
            // From where the tab is going, so quick Backspaces on a slow fs are not lost.
            Action::Parent => {
                // The root of an archive opened from inside another: back into that one.
                if let Some(back) = self.nested.get(&target).cloned() {
                    let name = target.file_name().map(|n| n.to_string_lossy().into_owned());
                    return self.load(side, back, name);
                }
                if let Some((path, focus)) = panel::parent_of(&target) {
                    return self.load(side, path, Some(focus));
                }
            }
            Action::Root => return self.load(side, "/".into(), None),
            // The zip the panel is in, or the one under the cursor.
            Action::ArchiveEncoding => {
                let is_zip = |p: &Path| {
                    Format::detect(&p.file_name().unwrap_or_default().to_string_lossy())
                        == Some(Format::Zip)
                };
                let on = (panel.current().filter(|e| !e.is_dir()))
                    .map(|e| panel.cwd().join(&e.os_name))
                    .filter(|z| is_zip(z));
                let zip = on.or_else(|| archive::split_path(panel.cwd()).map(|(a, _)| a));
                let Some(zip) = zip.filter(|z| is_zip(z)) else {
                    self.say(StatusKind::Error, fl!("archive-encoding-no-zip"));
                    return Task::none();
                };
                let now = archive::name_encoding(&zip);
                let label = |e: archive::NameEnc| match e {
                    archive::NameEnc::Auto => fl!("name-enc-auto"),
                    archive::NameEnc::Cp866 => fl!("name-enc-cp866"),
                    archive::NameEnc::Cp1251 => fl!("name-enc-cp1251"),
                    archive::NameEnc::Cp437 => fl!("name-enc-cp437"),
                    archive::NameEnc::Utf8 => fl!("name-enc-utf8"),
                };
                self.dialog = Some(Dialog::List {
                    kind: ListKind::NameEncoding,
                    side,
                    cursor: (archive::NameEnc::ALL.iter().position(|&e| e == now)).unwrap_or(0),
                    items: (archive::NameEnc::ALL.iter())
                        .map(|&e| ListItem {
                            label: label(e),
                            path: zip.clone(),
                            kind: Item::Dir,
                            icon: String::new(),
                        })
                        .collect(),
                });
            }
            Action::OpenTrash => match trashcan::home() {
                Ok(path) => return self.load(side, path, None),
                Err(e) => self.say(StatusKind::Error, crate::why::text(&e)),
            },
            Action::Restore => {
                let paths = panel.targets();
                if paths.is_empty() || !trashcan::is_top(panel.cwd()) {
                    self.say(StatusKind::Error, fl!("why-not-in-trash"));
                    return Task::none();
                }
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            (paths.iter())
                                .filter_map(|p| {
                                    let e = trashcan::restore(p).err()?;
                                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                                    Some(format!("{name}: {}", crate::why::text(&e)))
                                })
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |bad| cosmic::Action::App(Message::Restored(bad)),
                );
            }
            // Off the UI thread: finding the drives' trashes stats every mount.
            Action::EmptyTrash => {
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(trashcan::everything)
                            .await
                            .unwrap_or_else(|e| Err(std::io::Error::other(e)))
                            .map_err(|e| crate::why::text(&e))
                    },
                    move |r| cosmic::Action::App(Message::TrashListed(side, r)),
                );
            }
            Action::Mark => {
                panel.toggle_mark();
                // TC: Space on a dir also counts its size (not in archives: not real dirs).
                let dir = panel
                    .current()
                    .filter(|e| e.is_dir() && e.name != PARENT && panel.dir_size(e).is_none())
                    .map(|e| (panel.cwd().to_path_buf(), e.os_name.clone()));
                if let Some((cwd, name)) =
                    dir.filter(|(cwd, n)| !loading && !inside_archive(&cwd.join(n)))
                {
                    let stop = self.panes[side].active().counting.clone();
                    let count = count_dirs(tab, cwd, vec![name], stop);
                    return Task::batch([count, self.reveal(side, tab)]);
                }
            }
            // TC Alt+Shift+Enter: the size of every dir here, without marking.
            Action::CountDirs => {
                let cwd = panel.cwd().to_path_buf();
                if loading || inside_archive(&cwd.join("x")) {
                    return Task::none();
                }
                let names = panel
                    .entries()
                    .iter()
                    .filter(|e| e.is_dir() && e.name != PARENT && panel.dir_size(e).is_none())
                    .map(|e| e.os_name.clone())
                    .collect();
                let stop = self.panes[side].active().counting.clone();
                return count_dirs(tab, cwd, names, stop);
            }
            Action::MarkDown => panel.toggle_mark_and_move(1),
            Action::MarkUp => panel.toggle_mark_and_move(-1),
            Action::Invert => panel.invert(),
            Action::RestoreMarks => panel.restore_marks(),
            Action::ShowAll => panel.set_show(Show::All),
            Action::ShowPrograms => panel.set_show(Show::Programs),
            Action::MarkExt => panel.mark_ext(true),
            Action::UnmarkExt => panel.mark_ext(false),
            Action::MarkName => panel.mark_like(Like::Name, true),
            Action::UnmarkName => panel.mark_like(Like::Name, false),
            Action::MarkNameExt => panel.mark_like(Like::NameExt, true),
            Action::UnmarkNameExt => panel.mark_like(Like::NameExt, false),
            Action::SaveMarks => panel.keep_marks(),
            Action::NextMarked => panel.goto_marked(true),
            Action::PrevMarked => panel.goto_marked(false),
            Action::ShowOnlyMarked => panel.show_only_marked(),
            Action::LoadMarksFromClip => {
                return cosmic::iced::clipboard::read()
                    .map(move |t| cosmic::Action::App(Message::MarksText(side, t)));
            }
            // TC: the dir / archive under the cursor (`..`: the parent) in that panel; on a file,
            // this panel's dir.
            Action::OpenLeft | Action::OpenRight => {
                let to = usize::from(action == Action::OpenRight);
                let path = match panel.drop_target(panel.cursor()) {
                    Some(p) => p,
                    // On a file in this very panel: nothing (reading its dir again would drop
                    // search results / a branch).
                    None if to == side => return Task::none(),
                    None => panel.cwd().to_path_buf(),
                };
                return self.load(to, path, None);
            }
            Action::Occupied => {
                // Nothing marked, on `..`: the dir itself (TC).
                let mut targets = panel.targets();
                if targets.is_empty() {
                    targets.push(panel.cwd().to_path_buf());
                }
                if loading || targets.iter().any(|p| inside_archive(p)) {
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("counting"));
                // The tab's: leaving the dir stops it, as Space's counts.
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            shagoff_core::props::usage(&targets, &stop)
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    |u| cosmic::Action::App(Message::Occupied(u)),
                );
            }
            Action::TestArchives => {
                // One at a time; not on the rows of a dir being left.
                if self.testing || loading {
                    return Task::none();
                }
                // Dirs by the panel's rows, not a `stat` here (a dir may be named `x.zip`).
                let dirs: std::collections::HashSet<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| panel.cwd().join(&e.os_name))
                    .collect();
                let archives: Vec<PathBuf> = (panel.targets().into_iter())
                    .filter(|p| !inside_archive(p) && !dirs.contains(p))
                    .filter(|p| {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        archive::Format::detect(&name).is_some()
                    })
                    .collect();
                if archives.is_empty() {
                    self.say(StatusKind::Error, fl!("test-no-archives"));
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("testing-archives"));
                self.testing = true;
                // The tab's: leaving the dir stops it.
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            // Stopped (`Interrupted`): no more, and that one is not "damaged".
                            (archives.into_iter())
                                .map_while(|a| match archive::test(&a, &stop) {
                                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => None,
                                    r => Some((a, r.map_err(|e| crate::why::text(&e)))),
                                })
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |r| cosmic::Action::App(Message::Tested(r)),
                );
            }
            Action::ChecksumVerify => {
                if self.testing || loading {
                    return Task::none();
                }
                // Dirs by the panel's rows, not a `stat` here (as Alt+Shift+F9).
                let dirs: std::collections::HashSet<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| panel.cwd().join(&e.os_name))
                    .collect();
                let sums: Vec<PathBuf> = (panel.targets().into_iter())
                    .filter(|p| !inside_archive(p) && !dirs.contains(p))
                    .filter(|p| {
                        checksum::is_sum_file(&p.file_name().unwrap_or_default().to_string_lossy())
                    })
                    .collect();
                if sums.is_empty() {
                    self.say(StatusKind::Error, fl!("checksum-no-files"));
                    return Task::none();
                }
                self.say(StatusKind::Busy, fl!("checksum-verifying"));
                self.testing = true;
                let stop = self.panes[side].active().counting.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            (sums.into_iter())
                                .map_while(|s| match checksum::verify(&s, &stop) {
                                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => None,
                                    r => Some((s, r.map_err(|e| crate::why::text(&e)))),
                                })
                                .collect()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |r| cosmic::Action::App(Message::Verified(r)),
                );
            }
            Action::TreePanel => {} // `tree_act`
            // Taken at the top of `act`.
            Action::EditPath => {}
            Action::TreeDialog => {
                let hidden = panel.show_hidden();
                let cwd = panel.cwd().to_path_buf();
                // A dir on disk: inside an archive, the archive's own (a nested one: where the
                // outer archive lies, not its temp copy).
                let cwd = self.shown(&cwd);
                let here = match archive::split_path(&cwd) {
                    Some((a, _)) => a.parent().map_or(cwd.clone(), Path::to_path_buf),
                    None => cwd,
                };
                let root = PathBuf::from("/");
                let id = self.next_id();
                self.dialog = Some(Dialog::Tree {
                    side,
                    id,
                    tree: shagoff_core::tree::Tree::new(&root),
                    hidden,
                });
                let dirs = shagoff_core::tree::chain(&root, &here);
                return tree_read(id, dirs, hidden, Some(here));
            }
            Action::ContextMenu => {
                let ctx = match panel.current() {
                    Some(e) if e.name != PARENT => crate::menu::Ctx::entry(e, panel.cwd()),
                    _ => crate::menu::Ctx::Dir,
                };
                let items = (crate::menu::context_rows(ctx).into_iter())
                    .map(|(label, act)| ListItem {
                        label,
                        path: PathBuf::new(),
                        kind: act.map_or(Item::Sep, Item::Act),
                        icon: String::new(),
                    })
                    .collect();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Context,
                    side,
                    cursor: 0,
                    items,
                });
                return Task::none();
            }
            Action::SelectAll => panel.mark_all(true),
            Action::UnselectAll => panel.mark_all(false),
            Action::ToggleHidden => {
                let on = !self.config.show_hidden;
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_show_hidden(h, on) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.show_hidden = on,
                }
                return self.apply_hidden();
            }
            Action::View if self.config.internal_viewer => return self.view_current(side),
            Action::View | Action::Edit => {
                let cwd = panel.cwd().to_path_buf();
                let current = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| e.os_name.clone());
                let current = current.map(|os| located(&cwd, &os));
                if let Some((os_name, (arc, inner))) =
                    current.and_then(|(dir, own)| Some((own, archive::split_path(&dir)?)))
                {
                    // A link entry would open (and on "update", overwrite) whatever it points to.
                    let link = panel.current().is_some_and(|e| e.is_link);
                    if action == Action::Edit && link {
                        self.say(StatusKind::Error, fl!("archive-edit-link"));
                        return Task::none();
                    }
                    let how = if action == Action::Edit {
                        How::Edit(self.config.editor.clone())
                    } else {
                        How::Run(self.config.viewer.clone(), &["xdg-open"])
                    };
                    return self.open_from_archive(side, arc, inner, os_name, how);
                }
                let t = self.panes[side].active_mut();
                let panel = &t.panel;
                let file = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| (panel.cwd().join(&e.os_name), e.name.clone()));
                if let Some((file, name)) = file {
                    if let Some(why) = unopenable(&file, &name) {
                        self.say(StatusKind::Error, why);
                        return Task::none();
                    }
                    let argv = if action == Action::View {
                        launch::command(&self.config.viewer, &["xdg-open"], &file)
                    } else {
                        launch::command(&self.config.editor, &["cosmic-edit"], &file)
                    };
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(
                            StatusKind::Error,
                            fl!("open-failed", err = crate::why::text(&err)),
                        );
                    }
                }
            }
            // Opened by `dialog_for` above.
            Action::Drives(_) => {}
            Action::QuickSearch(_)
            | Action::QuickFilter
            | Action::Disconnect
            | Action::Help
            | Action::About
            | Action::Settings
            | Action::F12
            | Action::SwitchIgnoreList
            | Action::SwitchWatchDirs
            | Action::SwitchDirSort
            | Action::SwitchTypeColors
            | Action::ToggleDividerLine
            | Action::ToggleHumanSizes
            | Action::ToggleActiveFrame
            | Action::ToggleTerminal
            | Action::TermFull
            | Action::PickShell
            | Action::Donate
            | Action::OpenWith
            | Action::CopyNames
            | Action::CopyPaths
            | Action::CopySrcPath
            | Action::CopyTrgPath
            | Action::CopyDirsOfFiles
            | Action::CopyDetails
            | Action::CopyDetailsHdr
            | Action::CmdName
            | Action::CmdPath
            | Action::CmdCwd
            | Action::CmdPrevious
            | Action::CmdHistory
            | Action::TabRename => {} // handled above
            Action::Connect => {} // always a dialog
            Action::HistoryBack | Action::HistoryForward => {
                let step = |h: &mut History| {
                    if action == Action::HistoryBack {
                        h.back()
                    } else {
                        h.forward()
                    }
                };
                // Stepped in the copy a locked tab opens, so the locked one keeps its place.
                if step(&mut self.panes[side].active().history.clone()).is_some() {
                    self.leave_locked(side);
                    if let Some(path) = step(&mut self.panes[side].active_mut().history) {
                        return self.load(side, path, None);
                    }
                }
            }
            // Reached only when `dialog_for` found no pair of files.
            Action::CompareFiles => {
                let in_archive =
                    |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
                let msg = if in_archive(side) || in_archive(1 - side) {
                    fl!("diff-in-archive")
                } else {
                    fl!("diff-pick-two")
                };
                self.say(StatusKind::Error, msg);
            }
            Action::CompareLists | Action::CompareListsSubdirs | Action::CompareListsDiffOnly => {
                // Rows an earlier «only what differs» hid are compared too.
                for s in 0..2 {
                    let panel = &mut self.panes[s].active_mut().panel;
                    if panel.only_marked() {
                        panel.set_show(panel.show().clone());
                    }
                }
                let (l, r) = shagoff_core::sync::compare_lists(
                    self.panes[0].active().panel.entries(),
                    self.panes[1].active().panel.entries(),
                    action != Action::CompareLists,
                );
                // Nothing to mark must not look like a key that did nothing.
                if l.is_empty() && r.is_empty() {
                    self.say(StatusKind::Info, fl!("compare-identical"));
                }
                for (s, names) in [(0, &l), (1, &r)] {
                    let panel = &mut self.panes[s].active_mut().panel;
                    panel.mark_names(names);
                    if action == Action::CompareListsDiffOnly {
                        panel.hide_unmarked();
                    }
                }
                return Task::none();
            }
            Action::Branch | Action::BranchSel => return self.branch(side, action),
            Action::QuickView => {
                match self.lister.as_deref() {
                    Some(l) if l.quick.is_some() => self.lister = None,
                    Some(_) => {} // the full viewer is up: not from there
                    None => {
                        let mut l = Lister::new(side, String::new(), 0, self.config.lister_size());
                        // Nothing yet: `quick_follow` loads it.
                        l.quick = Some((PathBuf::new(), 0, std::time::UNIX_EPOCH));
                        self.lister = Some(Box::new(l));
                    }
                }
                return Task::none();
            }
            Action::SyncChangeDir => {
                self.sync_dirs = !self.sync_dirs;
                let note = match self.sync_dirs {
                    true => fl!("sync-change-dir-on"),
                    false => fl!("sync-change-dir-off"),
                };
                self.say(StatusKind::Info, note);
                return Task::none();
            }
            Action::MatchSrc => return self.load(1 - side, target, None),
            Action::SwapPanes => {
                self.panes.swap(0, 1);
                self.space.swap(0, 1);
                self.search = None;
                return Task::batch([self.restore_scroll(0), self.restore_scroll(1)]);
            }
            // Opened by `dialog_for` above.
            Action::HistoryList | Action::Hotlist => {}
            Action::ClipCopy | Action::ClipCut => {
                let paths = panel.targets();
                if !paths.is_empty() {
                    let kind = if action == Action::ClipCut {
                        ClipKind::Cut
                    } else {
                        ClipKind::Copy
                    };
                    // Through an archive (a stat, not the name: `backup.zip/` may be a real dir).
                    let inner =
                        (paths.iter()).any(|p| p.parent().and_then(archive::split_path).is_some());
                    return clip::put(kind, &paths, inner);
                }
            }
            Action::ClipPaste => return clip::take(),
            Action::Copy
            | Action::CopySame
            | Action::Link
            | Action::Properties
            | Action::Move
            | Action::Rename
            | Action::MultiRename
            | Action::Pack
            | Action::MoveToArchive
            | Action::Unpack
            | Action::ChecksumCreate
            | Action::FindFiles
            | Action::SyncDirs
            | Action::Mkdir
            | Action::NewFile
            | Action::Delete
            | Action::DeletePermanent
            | Action::SaveMarksToFile
            | Action::LoadMarksFromFile
            | Action::SaveDetails
            | Action::SaveDetailsHdr
            | Action::SplitFile => {}
            // Opened by `dialog_for` above.
            Action::SelectGroup | Action::UnselectGroup | Action::ShowMask => {}
            // Also the drive list: a stick plugged in shows up (TC rereads drives too).
            Action::Reload => {
                // Where the tab is going (results fed, a dir entered, not listed yet), not where
                // it was: that would drop the results / undo the step.
                let cwd = self.panes[side].active().target();
                // Pictures are known by mtime: a chmod or `cp -p` is seen only on Ctrl+R.
                self.thumbs.clear();
                self.refresh_mounts();
                let volumes = self.refresh_volumes(true);
                return Task::batch([self.reload(side, cwd, None), volumes]);
            }
            Action::TabOpen | Action::TabOpenOther => {
                let path = self.tab_target(side);
                let to = if action == Action::TabOpen {
                    side
                } else {
                    1 - side
                };
                // TC: in the sort of the tab it was opened from.
                let sort = self.panes[side].active().panel.sort();
                let mut tab = self.new_tab(path.clone());
                tab.panel.set_sort_order(sort);
                self.panes[to].open_after(tab);
                return self.load(to, path, None);
            }
            Action::TabCopyOther | Action::TabMoveOther => {
                let tab = if action == Action::TabCopyOther {
                    let id = self.next_id();
                    Some(self.panes[side].active().duplicate(id))
                } else {
                    let i = self.panes[side].active_index();
                    self.panes[side].take(i)
                };
                let Some(tab) = tab else {
                    self.say(StatusKind::Error, fl!("tab-last"));
                    return Task::none();
                };
                self.panes[1 - side].open_after(tab);
                self.active = 1 - side;
                let mut tasks = vec![self.tab_switched(1 - side)];
                if action == Action::TabMoveOther {
                    tasks.push(self.tab_switched(side));
                }
                return Task::batch(tasks);
            }
            Action::TabRoam => {
                let t = self.panes[side].active_mut();
                t.home = match t.home {
                    Some(_) => None,
                    None => Some(t.panel.cwd().to_path_buf()),
                };
                t.locked = false;
                return Task::none();
            }
            Action::TabLock => {
                let t = self.panes[side].active_mut();
                t.locked = !t.locked;
                t.home = None;
                // A scan into another dir would still land and move it.
                if t.locked && t.target() != t.panel.cwd() {
                    t.pending = None;
                }
                return Task::none();
            }
            Action::CloseOtherTabs => {
                self.panes[side].retain(Tab::pinned);
                return Task::none();
            }
            // TC asks before closing locked ones; here they stay, and it is said.
            Action::CloseAllTabs => {
                self.panes[side].retain(Tab::pinned);
                if self.panes[side].items().len() > 1 {
                    self.say(StatusKind::Info, fl!("tabs-locked-kept"));
                }
                return Task::none();
            }
            Action::CloseDuplicateTabs => {
                self.panes[side].close_duplicates(Tab::target, Tab::pinned);
                return Task::none();
            }
            Action::LockAllTabs | Action::RoamAllTabs | Action::UnlockAllTabs => {
                for t in self.panes[side].items_mut() {
                    t.locked = action == Action::LockAllTabs;
                    // A tab moving about keeps its own dir; another gets the one it is going to.
                    t.home = match action {
                        Action::RoamAllTabs => Some(t.home.clone().unwrap_or_else(|| t.target())),
                        _ => None,
                    };
                    // As Lock on one tab: a scan into another dir would still land and move it.
                    if t.locked && t.target() != t.panel.cwd() {
                        t.pending = None;
                    }
                }
                return Task::none();
            }
            Action::GoToLockedDir => {
                let Some(home) = self.panes[side].active().home.clone() else {
                    self.say(StatusKind::Error, fl!("tab-not-roaming"));
                    return Task::none();
                };
                return self.load(side, home, None);
            }
            // Here only when `dialog_for` found no parts under the cursor.
            Action::CombineFiles => {
                self.say(StatusKind::Error, fl!("combine-not-part"));
                return Task::none();
            }
            Action::NewFromTemplate => {
                let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
                let dir = templates::dir(&self.home, config);
                let files = templates::list(&dir, &self.home);
                if files.is_empty() {
                    let dir = dir.display().to_string();
                    self.say(StatusKind::Error, fl!("templates-none", dir = dir));
                    return Task::none();
                }
                let items = (files.into_iter())
                    .map(|path| ListItem {
                        label: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path,
                        kind: Item::Dir,
                        icon: String::new(),
                    })
                    .collect();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Templates,
                    side,
                    cursor: 0,
                    items,
                });
                return Task::none();
            }
            Action::TabsList => {
                let tabs = self.panes[side].items();
                let items = (tabs.iter().enumerate())
                    .map(|(i, t)| ListItem {
                        label: format!("{}. {}", i + 1, t.title()),
                        path: self.shown(&t.target()),
                        kind: Item::Dir,
                        icon: String::new(),
                    })
                    .collect();
                self.dialog = Some(Dialog::List {
                    kind: ListKind::Tabs,
                    side,
                    cursor: self.panes[side].active_index(),
                    items,
                });
                return Task::none();
            }

            // Ctrl+T / Ctrl+Shift+T (staying in this one) / in the other panel (the keys staying here).
            Action::NewTab | Action::NewTabBg | Action::NewTabOther => {
                let id = self.next_id();
                let mut copy = self.panes[side].active().duplicate(id);
                // TC: the new tab shows the whole dir.
                copy.panel.set_filter(None);
                copy.panel.set_show(Show::All);
                let here = self.panes[side].active_index();
                return match action {
                    Action::NewTabOther => {
                        self.panes[1 - side].open_after(copy);
                        self.tab_switched(1 - side)
                    }
                    Action::NewTabBg => {
                        self.panes[side].open_after(copy);
                        self.panes[side].select(here);
                        Task::none()
                    }
                    _ => {
                        self.panes[side].open_after(copy);
                        self.restore_scroll(side)
                    }
                };
            }
            Action::CloseTab => {
                if self.panes[side].active().pinned() {
                    self.say(StatusKind::Error, fl!("tab-locked"));
                    return Task::none();
                }
                let i = self.panes[side].active_index();
                if !self.panes[side].close(i) {
                    if self.config.last_tab_close == LastTab::Home {
                        let home = match &self.config.home_dir {
                            Some(d) => session::existing_dir(
                                &session::expand_home(d, &self.home),
                                &self.home,
                            ),
                            None => self.home.clone(),
                        };
                        return self.load(side, home, None);
                    }
                    return Task::none();
                }
                return self.tab_switched(side);
            }
            Action::NextTab => {
                self.panes[side].next();
                return self.tab_switched(side);
            }
            Action::PrevTab => {
                self.panes[side].prev();
                return self.tab_switched(side);
            }
            Action::MoveTabLeft | Action::MoveTabRight => {
                let tabs = &mut self.panes[side];
                let (i, n) = (tabs.active_index(), tabs.items().len());
                let to = if action == Action::MoveTabLeft {
                    i.checked_sub(1)
                } else {
                    Some(i + 1).filter(|&j| j < n)
                };
                if let Some(to) = to {
                    tabs.move_to(i, to);
                }
            }
        }
        self.reveal(side, tab)
    }
}
