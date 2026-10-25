//! Messages → state: `App::handle`, the one big `match` over `Message`.

use super::*;

impl App {
    pub(super) fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            // The focused terminal took these (F8 in mc, Ctrl+R in bash, Esc in vim): a key a
            // text field leaves to us, `route_event` passes on at any status — not this one's.
            Message::FieldKey(_) | Message::Escape(_) | Message::FeedKey(_)
                if self.term_focused && self.dialog.is_none() => {}
            Message::TermFocused(on) => self.term_focused = on,
            Message::Key(action) if self.viewer() && self.dialog.is_none() && !self.busy() => {
                if let Some(task) = self.lister_action(action) {
                    return task;
                }
                if matches!(action, Action::RunTerminal | Action::UserMenu(_)) {
                    return Task::none(); // not the panel's file from under the viewer
                }
                return self.act(self.active, action);
            }
            Message::Key(action) => {
                // The drop menu as TC's: ↑ / ↓ over Copy, Move, Link, Cancel; Enter picks.
                if let Some(Dialog::DropMenu {
                    cursor, sources, ..
                }) = &mut self.dialog
                {
                    let ops = dialogs::drop_ops(sources);
                    let last = ops.len();
                    match action {
                        Action::Up => *cursor = cursor.checked_sub(1).unwrap_or(last),
                        Action::Down => *cursor = (*cursor + 1) % (last + 1),
                        Action::Enter => {
                            let op = ops.get(*cursor).copied();
                            return self.handle(op.map_or(Message::DialogCancel, Message::DropAs));
                        }
                        _ => {}
                    }
                    return Task::none();
                }
                if let Some(Dialog::List {
                    kind,
                    cursor,
                    items,
                    ..
                }) = &self.dialog
                    && *kind == ListKind::Hotlist
                {
                    let row = items.get(*cursor).map(|it| it.kind);
                    match (action, row) {
                        (Action::Delete | Action::DeletePermanent, Some(Item::Hot(i))) => {
                            return self.hotlist_remove(i);
                        }
                        (Action::Right, Some(Item::Sub(_))) => {
                            let i = *cursor;
                            return self.pick(i);
                        }
                        (Action::Left | Action::Parent, _) if !self.hot_sub.is_empty() => {
                            self.hotlist_back();
                            return Task::none();
                        }
                        _ => {}
                    }
                }
                if let Some(Dialog::Tree {
                    tree, hidden, id, ..
                }) = &mut self.dialog
                {
                    const PAGE: isize = 10;
                    match action {
                        Action::Up => tree.move_cursor(-1),
                        Action::Down => tree.move_cursor(1),
                        Action::PageUp => tree.move_cursor(-PAGE),
                        Action::PageDown => tree.move_cursor(PAGE),
                        Action::Home => tree.move_cursor(-isize::MAX / 2),
                        Action::End => tree.move_cursor(isize::MAX / 2),
                        // → opens it (read in the background), then steps in; ← back out.
                        Action::Right => {
                            if let Some(dir) = tree.step_in() {
                                return tree_read(*id, vec![dir], *hidden, None);
                            }
                        }
                        Action::Left | Action::Parent => tree.step_out(),
                        Action::Enter => return self.submit_dialog(),
                        _ => return Task::none(),
                    }
                    return list_snap(tree.cursor, tree.rows.len());
                }
                if let Some(Dialog::List { cursor, items, .. }) = &mut self.dialog {
                    // Separators are skipped, as in a menu.
                    let step = |from: usize, up: bool| {
                        let mut i = from;
                        loop {
                            i = match (up, i) {
                                (true, 0) => return from,
                                (true, _) => i - 1,
                                (false, _) if i + 1 >= items.len() => return from,
                                (false, _) => i + 1,
                            };
                            if items[i].kind != Item::Sep {
                                return i;
                            }
                        }
                    };
                    match action {
                        Action::Up => *cursor = step(*cursor, true),
                        Action::Down => *cursor = step(*cursor, false),
                        Action::Enter => {
                            let i = *cursor;
                            return self.pick(i);
                        }
                        _ => return Task::none(),
                    }
                    return list_snap(*cursor, items.len());
                }
                if let Some(Dialog::Diff(_)) = &self.dialog {
                    return match action {
                        Action::Down => self.diff_step(1),
                        Action::Up => self.diff_step(-1),
                        _ => Task::none(),
                    };
                }
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    match action {
                        Action::Up => {
                            f.cursor = f.cursor.saturating_sub(1);
                            f.in_list = true;
                        }
                        Action::Down => {
                            let shown = f.results.len().min(dialogs::FIND_SHOWN);
                            f.cursor = (f.cursor + 1).min(shown.saturating_sub(1));
                            f.in_list = true;
                        }
                        Action::Enter => {
                            let i = f.cursor;
                            return self.find_pick(i);
                        }
                        Action::SwitchPane => return cosmic::iced::widget::operation::focus_next(),
                        _ => {}
                    }
                    return Task::none();
                }
                // The field lets ↑ / ↓ through as plain keys.
                if let Some(task) = self.mask_history(action) {
                    return task;
                }
                if let Some(d) = &self.dialog {
                    // Modal: panels must not move. Enter confirms a dialog without a text field.
                    if action == Action::Enter
                        && matches!(
                            d,
                            Dialog::ConfirmDelete { .. }
                                | Dialog::Props(_)
                                | Dialog::Checksum { .. }
                        )
                    {
                        return self.submit_dialog();
                    }
                    // Through its buttons and fields, as in TC: arrows and Tab (a field keeps ← →
                    // for itself); Enter presses the focused button (the button takes it).
                    use cosmic::iced::widget::operation::{focus_next, focus_previous};
                    return match action {
                        Action::Down | Action::Right | Action::SwitchPane => focus_next(),
                        Action::Up | Action::Left => focus_previous(),
                        _ => Task::none(),
                    };
                }
                if self.busy() {
                    return Task::none(); // panels wait for the running operation's dialog
                }
                if let Some(s) = &self.search {
                    let (side, filter, text) = (s.side, s.filter, s.text.clone());
                    match action {
                        Action::Up | Action::Down if !filter => {
                            let p = &self.panes[side].active().panel;
                            let n = p.entries().len();
                            let down = action == Action::Down;
                            let from = if down {
                                p.cursor() + 1
                            } else {
                                (p.cursor() + n).saturating_sub(1) // empty list: none found
                            };
                            if let Some(i) = p.find(&text, from, down) {
                                let t = self.panes[side].active_mut();
                                t.panel.set_cursor(i);
                                let tab = t.id;
                                return self.reveal(side, tab);
                            }
                            return Task::none();
                        }
                        Action::Up | Action::Down => {} // filter: plain cursor move, field stays
                        // The field lost focus (a click): Alt+letter goes on with its text.
                        Action::QuickSearch(_) if !filter => {}
                        _ => self.search = None, // any other key closes the field and acts
                    }
                }
                // Only the Enter key runs a typed line (the field lost focus to a click); a double
                // click or quick search's Enter opens the entry.
                if matches!(action, Action::Enter | Action::RunTerminal)
                    && self.cmd_ready()
                    && !self.cmdline.trim().is_empty()
                {
                    return self.cmd_run(action == Action::RunTerminal);
                }
                return self.act(self.active, action);
            }
            Message::Listed {
                tab,
                generation,
                path,
                focus,
                result,
                space,
            } => {
                // The tab may have moved (Ctrl+U): find it by id; `side` is only where it started.
                let Some(side) = self.side_of(tab) else {
                    return Task::none(); // tab was closed
                };
                let Some(t) = self.panes[side]
                    .items_mut()
                    .iter_mut()
                    .find(|t| t.id == tab)
                else {
                    return Task::none();
                };
                if t.pending.as_ref().map(|(g, _)| *g) != Some(generation) {
                    return Task::none(); // stale: the user has moved on
                }
                t.pending = None;
                // Ctrl+B's rows are in: the branch is read.
                if t.branching
                    .take_if(|_| t.branch && t.results.is_some())
                    .is_some()
                    && (self.status.as_ref()).is_some_and(|s| s.kind == StatusKind::Busy)
                {
                    self.status = None;
                }
                let Some(t) = (self.panes[side].items_mut().iter_mut()).find(|t| t.id == tab)
                else {
                    return Task::none();
                };
                match result {
                    Ok(entries) => {
                        t.error = None;
                        t.history.visit(&path); // no-op for a rescan of the current entry
                        let moved = path != t.panel.cwd();
                        if moved {
                            t.counting.store(true, Ordering::Relaxed);
                            t.counting = Arc::default();
                        }
                        t.panel.set_listing(path, entries, focus.as_deref());
                        let shown = self.panes[side].active().id == tab;
                        if shown {
                            self.refresh_mounts();
                            self.space[side] = space;
                        }
                        // The tree's tab went elsewhere (Alt+F10, Ctrl+D…): the tree opened there.
                        let tree = self.panes[side].active().tree.is_some();
                        if shown && moved && tree {
                            return Task::batch([self.reveal(side, tab), self.tree_open(side)]);
                        }
                        return self.reveal(side, tab);
                    }
                    Err(e) => {
                        // An Alt+← / Alt+→ step that failed: history back on the dir shown.
                        let shown = t.panel.cwd().to_path_buf();
                        t.history.seat(&shown);
                        let ask = archive::needs_password(&e)
                            .then(|| archive::split_path(&path))
                            .flatten();
                        t.error = Some(fl!(
                            "list-failed",
                            path = path.display().to_string(),
                            err = crate::why::message(&e)
                        ));
                        // A 7z with encrypted names: ask, then open it again.
                        if let Some((arc, _)) = ask
                            && self.dialog.is_none()
                        {
                            self.dialog = Some(Dialog::Password {
                                wrong: archive::password(&arc).is_some(),
                                archive: arc,
                                password: String::new(),
                                reply: None,
                                open: Some((side, path)),
                            });
                            return widget::text_input::focus(self.input_id.clone());
                        }
                    }
                }
            }
            Message::Click(side, i) => {
                self.click_pane(side);
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                if self.mods.control() {
                    t.panel.toggle_mark();
                }
                let tab = t.id;
                return self.reveal(side, tab); // a half-visible row scrolls fully in
            }
            Message::RightClick(side, i) if self.config.right_marks => {
                self.click_pane(side);
                self.ctx_row = true;
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                let tab = t.id;
                return Task::batch([self.reveal(side, tab), self.right_press(side, Some(i))]);
            }
            Message::RightEmpty(side) if self.config.right_marks => {
                self.click_pane(side);
                // The same press on a row came first: that row's hold stands.
                if !std::mem::take(&mut self.ctx_row) {
                    return self.right_press(side, None);
                }
            }
            Message::RowEnter(side, i) => {
                let Some(h) = &mut self.right_hold else {
                    return Task::none();
                };
                let t = self.panes[side].active_mut();
                let Some(start) = h.row.filter(|_| h.side == side && h.tab == t.id) else {
                    return Task::none();
                };
                let (on, last) = match h.drag {
                    Some(d) => d,
                    None if i == start => return Task::none(),
                    // Off the pressed row: a drag, marking as that row would have become.
                    None => {
                        let e = t.panel.entries().get(start);
                        (!e.is_some_and(|e| t.panel.is_marked(e)), start)
                    }
                };
                h.drag = Some((on, i));
                // From the last row reached: a quick move skips rows' enter events.
                t.panel.mark_range(last, i, on);
                t.panel.set_cursor(i);
                let tab = t.id;
                return self.reveal(side, tab);
            }
            Message::RightHeld(n) => {
                if let Some(h) = &mut self.right_hold
                    && h.n == n
                    && h.drag.is_none()
                {
                    h.armed = true;
                    self.ctx_entry = h.row.is_some();
                }
            }
            Message::RightUp => {
                let Some(h) = self.right_hold.take() else {
                    return Task::none();
                };
                if h.drag.is_some() {
                    return Task::none(); // marked while dragging: done
                }
                let t = self.panes[h.side].active_mut();
                // Only where it was pressed: not after a tab switch, or with the cursor moved.
                if t.id != h.tab || h.row.is_none_or(|r| r != t.panel.cursor()) {
                    return Task::none();
                }
                if !h.armed {
                    t.panel.toggle_mark(); // a quick click: TC marks the row
                } else if !t.panel.current().is_some_and(|e| t.panel.is_marked(e)) {
                    // The menu opens now, as a right click's at once: about the unmarked row only.
                    t.panel.mark_all(false);
                }
            }
            Message::RightClick(side, i) => {
                self.search = None;
                self.active = side;
                (self.ctx_entry, self.ctx_row) = (true, true);
                let t = self.panes[side].active_mut();
                t.panel.set_cursor(i);
                // As in Explorer: the menu is about what was clicked. A marked row keeps the marks
                // (the menu is for all of them); an unmarked one drops them.
                let clicked_marked = t.panel.current().is_some_and(|e| t.panel.is_marked(e));
                if !clicked_marked {
                    t.panel.mark_all(false);
                }
                let tab = t.id;
                return self.reveal(side, tab);
            }
            Message::OpenEntry => return self.act(self.active, Action::Enter),
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(a),
                ));
            }
            Message::RightEmpty(side) => {
                self.search = None;
                self.active = side;
                // The same press on a row came first: that row's menu stands.
                if !std::mem::take(&mut self.ctx_row) {
                    self.ctx_entry = false;
                }
            }
            Message::DoubleClick(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].active_mut().panel.set_cursor(i);
                return self.act(side, Action::Enter);
            }
            Message::PaneKey(side, action) => {
                self.search = None;
                self.active = side;
                return self.act(side, action);
            }
            Message::Scrolled(side, offset, height) => {
                // Wheel / scrollbar: the view moves freely, the cursor stays where it is.
                let t = self.panes[side].active_mut();
                t.offset = offset;
                t.height = height;
            }
            Message::Resized(side, size) => {
                // One list widget per pane: every tab shares its viewport size.
                let t = self.panes[side].active();
                // Brief: any size change regroups the columns, the cursor may leave the screen.
                let shrunk = size.height < t.height || t.brief || t.thumbs;
                for t in self.panes[side].items_mut() {
                    (t.width, t.height) = (size.width, size.height);
                }
                if shrunk {
                    let tab = self.panes[side].active().id;
                    return self.reveal(side, tab); // keep the cursor on screen
                }
            }
            Message::Thumb(path, mtime, pic) => {
                self.thumbs_busy -= 1;
                // Changed since it was asked: the next `want_thumbs` asks again.
                if let Some(slot) = self.thumbs.get_mut(&path)
                    && slot.0 == mtime
                {
                    slot.1 = pic;
                }
            }
            Message::BriefWheel(side, delta) => {
                let row_h = self.row_h();
                let t = self.panes[side].active_mut();
                // Touchpads send many small pixel steps: one column per row height of travel.
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        t.wheel += y;
                        let steps = (t.wheel / row_h).trunc();
                        t.wheel -= steps * row_h;
                        steps
                    }
                };
                let (rows, cols) = t.brief_grid(row_h, self.config.text_scale());
                let last = t.panel.entries().len().div_ceil(rows).saturating_sub(cols);
                t.col = if y > 0.0 {
                    t.col.saturating_sub(1)
                } else if y < 0.0 {
                    (t.col + 1).min(last)
                } else {
                    t.col
                };
            }
            Message::TabPress(side, i) => {
                self.tab_drag = Some((side, i));
                return self.update(Message::SelectTab(side, i));
            }
            Message::TabEnter(side, i) => {
                if let Some((s, from)) = &mut self.tab_drag
                    && *s == side
                    && *from != i
                {
                    self.panes[side].move_to(*from, i);
                    *from = i;
                }
            }
            Message::SelectTab(side, i) => {
                self.search = None;
                self.active = side;
                self.panes[side].select(i);
                return self.tab_switched(side);
            }
            Message::CloseTabAt(side, i) => {
                self.search = None;
                if self.panes[side].items().get(i).is_some_and(Tab::pinned) {
                    self.say(StatusKind::Error, fl!("tab-locked"));
                    return Task::none();
                }
                self.panes[side].close(i);
                return self.tab_switched(side);
            }
            Message::Modifiers(m) => {
                self.mods = m;
                // Shift let go (or the window left, which clears them): a held button is dropped
                // where it is — the release may never come.
                if !m.shift() {
                    self.tool_drop();
                }
            }
            Message::DialogInput(s) => {
                if let Some(input) = self.dialog.as_mut().and_then(Dialog::input_mut) {
                    *input = s;
                }
            }
            Message::MrInput(field, s) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    *match field {
                        MrField::Name => &mut m.rule.name,
                        MrField::Ext => &mut m.rule.ext,
                        MrField::Find => &mut m.rule.find,
                        MrField::Replace => &mut m.rule.replace,
                        MrField::Start => &mut m.start,
                        MrField::Step => &mut m.step,
                        MrField::Digits => &mut m.digits,
                    } = s;
                }
            }
            Message::MrInsert(tok) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.name.push_str(tok);
                    return Task::batch([
                        widget::text_input::focus(self.input_id.clone()),
                        widget::text_input::move_cursor_to_end(self.input_id.clone()),
                    ]);
                }
            }
            Message::MrHelp => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.help = !m.help;
                }
            }
            Message::MrCase(c) => {
                if let Some(Dialog::MultiRename(m)) = &mut self.dialog {
                    m.rule.case = c;
                }
            }
            Message::SumAlgo(a) => {
                if let Some(Dialog::Checksum { algo, .. }) = &mut self.dialog {
                    *algo = a;
                }
                let v = a.ext().to_string();
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_checksum_algo(h, v) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.checksum_algo = v,
                }
            }
            Message::PackFormat(f) => {
                if let Some(Dialog::Pack(p)) = &mut self.dialog {
                    p.format = f;
                    p.path = archive::with_format(&p.path, f);
                }
                self.save_pack_format(f);
            }
            Message::ArchivePassword(s) => {
                if let Some(Dialog::Password { password, .. }) = &mut self.dialog {
                    *password = s;
                }
            }
            Message::DateText(i, s) => {
                if let Some(Dialog::SetDate { text, change, .. }) = &mut self.dialog {
                    text[i] = s;
                    change[i] = true; // typed into: meant
                }
            }
            Message::DateChange(i, b) => {
                if let Some(Dialog::SetDate { change, .. }) = &mut self.dialog {
                    change[i] = b;
                }
            }
            Message::DateNow => {
                let now = format::date_edit(std::time::SystemTime::now(), &self.tz);
                if let Some(Dialog::SetDate { text, change, .. }) = &mut self.dialog {
                    (*text, *change) = ([now.clone(), now], [true, true]);
                }
            }
            Message::DateRecursive(b) => {
                if let Some(Dialog::SetDate { recursive, .. }) = &mut self.dialog {
                    *recursive = b;
                }
            }
            Message::PackPassword(again, s) => {
                if let Some(Dialog::Pack(p)) = &mut self.dialog {
                    *if again { &mut p.again } else { &mut p.password } = s;
                }
            }
            Message::Toggle(t) => match (&mut self.dialog, t) {
                (Some(Dialog::Pack(p)), Toggle::MoveAfter) => p.move_after = !p.move_after,
                (Some(Dialog::Pack(p)), Toggle::Separate) => p.separate = !p.separate,
                (Some(Dialog::Pack(p)), Toggle::Encrypt) => {
                    p.encrypt = !p.encrypt;
                    if p.encrypt {
                        return widget::text_input::focus(widget::Id::new("pack-password"));
                    }
                }
                (Some(Dialog::Pack(p)), Toggle::EncryptNames) => p.names = !p.names,
                (Some(Dialog::Unpack { own_dir, .. }), Toggle::OwnDir) => *own_dir = !*own_dir,
                (Some(Dialog::Checksum { separate, .. }), Toggle::SumSeparate) => {
                    *separate = !*separate
                }
                _ => {}
            },
            Message::FindInput(field, s) => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.in_list = false;
                    *match field {
                        FindField::Mask => &mut f.mask,
                        FindField::Dir => &mut f.dir,
                        FindField::Text => &mut f.text,
                        FindField::MinSize => &mut f.min_size,
                        FindField::MaxSize => &mut f.max_size,
                        FindField::Days => &mut f.days,
                        FindField::Attrs => &mut f.attrs,
                    } = s;
                }
            }
            Message::FindCase => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.case_sensitive = !f.case_sensitive;
                    f.in_list = false;
                }
            }
            Message::FindTextOn => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.find_text = !f.find_text;
                    f.in_list = false;
                }
            }
            Message::FindHelp => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.help = !f.help;
                }
            }
            Message::FindRegex => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.regex = !f.regex;
                    f.in_list = false;
                }
            }
            Message::FindNameRegex => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.name_regex = !f.name_regex;
                    f.in_list = false;
                }
            }
            Message::FindArchives => {
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    f.archives = !f.archives;
                    f.in_list = false;
                }
            }
            Message::PropsGroup(i) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && i < p.gids.len()
                {
                    p.group = Some(i);
                }
            }
            Message::PropsUser(i) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && i < p.uids.len()
                {
                    p.user = Some(i);
                }
            }
            Message::PropsBit(b) => {
                // In an archive the bits are only shown.
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && p.archive.is_none()
                {
                    if p.mixed & !p.touched & b != 0 {
                        p.mode |= b; // `?` → on
                    } else {
                        p.mode ^= b;
                    }
                    p.touched |= b;
                }
            }
            Message::PropsRecursive => {
                if let Some(Dialog::Props(p)) = &mut self.dialog {
                    p.recursive = !p.recursive;
                    p.mixed = if p.recursive { 0o7777 } else { p.own_mixed };
                }
            }
            Message::DirSize(tab, cwd, name, bytes) => {
                // By id: the tab may have moved to the other panel meanwhile (Ctrl+U, a drag).
                if let Some(side) = self.side_of(tab)
                    && let Some(t) = self.panes[side]
                        .items_mut()
                        .iter_mut()
                        .find(|t| t.id == tab)
                {
                    t.panel.set_dir_size(&cwd, name, bytes);
                }
            }
            Message::PropsUsage(id, usage) => {
                if let Some(Dialog::Props(p)) = &mut self.dialog
                    && p.id == id
                {
                    p.usage = usage;
                }
            }
            Message::PropsDone(side, failed) => {
                match failed.first() {
                    None => self.say(StatusKind::Info, fl!("props-changed")),
                    Some(err) => {
                        let msg = fl!("props-failed", n = failed.len(), err = err.clone());
                        self.say(StatusKind::Error, msg);
                    }
                }
                // Both: the other panel may show the same dir.
                let tasks: Vec<_> = [side, 1 - side]
                    .into_iter()
                    .map(|s| {
                        let dir = self.panes[s].active().target();
                        self.reload(s, dir, None)
                    })
                    .collect();
                return Task::batch(tasks);
            }
            Message::FindFeed => return self.find_feed(),
            Message::FeedKey(other) => {
                // As the button: nothing to feed, nothing done.
                if let Some(Dialog::Find(f)) = &self.dialog {
                    return match f.results.is_empty() {
                        true => Task::none(),
                        false => self.find_feed(),
                    };
                }
                if let Some(a) = other {
                    return self.update(Message::Key(a));
                }
            }
            Message::FindStart => return self.start_find(),
            Message::FindSubmit => {
                if let Some(Dialog::Find(f)) = &self.dialog
                    && f.in_list
                    && !f.results.is_empty()
                {
                    let i = f.cursor;
                    return self.find_pick(i);
                }
                return self.start_find();
            }
            Message::FindStop => {
                if let Some(Dialog::Find(f)) = &self.dialog
                    && let Some(s) = &f.stop
                {
                    s.store(true, Ordering::Relaxed);
                }
            }
            Message::Find(e) => self.on_find_event(e),
            Message::FindPick(i) => return self.find_pick(i),
            Message::SyncOpt(o) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog {
                    let flag = match o {
                        SyncOpt::Recursive => &mut s.recursive,
                        SyncOpt::Content => &mut s.content,
                        SyncOpt::IgnoreDate => &mut s.ignore_date,
                        SyncOpt::Mirror => &mut s.mirror,
                    };
                    *flag = !*flag;
                    // The rows must match the options they are synced with.
                    return self.start_compare();
                }
            }
            Message::SyncShow(kind) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && !s.hide.remove(&kind)
                {
                    s.hide.insert(kind);
                }
            }
            Message::SyncCompare => return self.start_compare(),
            Message::SyncMask(m) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog {
                    s.mask = m;
                    // The rows were for the old mask: nothing to run until compared again.
                    s.rows.clear();
                    s.confirm = false;
                }
            }
            Message::SyncStop => {
                if let Some(Dialog::Sync(s)) = &self.dialog
                    && let Some(f) = &s.running
                {
                    f.store(true, Ordering::Relaxed);
                }
            }
            Message::SyncCompared(id, rows) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && s.id == id
                {
                    s.rows = rows;
                    s.running = None;
                    s.confirm = false; // asked again for the new rows
                }
            }
            Message::SyncFlip(i) => {
                if let Some(Dialog::Sync(s)) = &mut self.dialog
                    && let Some(row) = s.rows.get_mut(i)
                {
                    row.dir = shagoff_core::sync::next_dir(row);
                    s.confirm = false;
                }
            }
            Message::SyncRun => return self.start_sync(),
            Message::DiffReady(id, out) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.id == id
                {
                    if let Ok(shagoff_core::diff::Outcome::Text(t)) = out.as_ref() {
                        d.widths = dialogs::DiffDlg::measure(t);
                        if t.stamps.is_some() {
                            d.stamps = t.stamps; // read from disk now
                        }
                        // A copied block makes its side unsaved only if it changed the text.
                        if let Some((to_right, before)) = d.copying.take()
                            && t.src != before
                        {
                            d.undo.push((before, d.dirty));
                            if to_right {
                                d.dirty.1 = true;
                            } else {
                                d.dirty.0 = true;
                            }
                        }
                    }
                    d.result = Some(out);
                    // A re-compare (options, a copied block) stays near where it was.
                    d.block = d.block.min(d.blocks().len().saturating_sub(1));
                    return self.diff_scroll();
                }
            }
            Message::DiffScrolled(side, x, y) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    // The follower reports the same offset back: stop there.
                    if (x - d.x).abs() < 0.5 && (y - d.offset).abs() < 0.5 {
                        return Task::none();
                    }
                    (d.x, d.offset) = (x, y);
                    // Scrolled away from the current block: the first one in view takes over.
                    let (top, bottom) = d.view_rows();
                    if let Some(t) = d.text() {
                        d.block = shagoff_core::diff::block_in_view(
                            &t.rows,
                            d.blocks(),
                            d.block,
                            top,
                            bottom,
                        );
                    }
                    let to = AbsoluteOffset {
                        x: Some(x),
                        y: Some(y),
                    };
                    return scrollable::scroll_to(d.scroll[1 - side].clone(), to);
                }
            }
            Message::DiffReread => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.result.is_some()
                    && !(d.dirty.0 || d.dirty.1)
                {
                    d.block = 0;
                    return self.start_diff();
                }
            }
            Message::DiffOpt(o) => {
                // Not while a compare runs: it may hold a copied block this would drop.
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(shown) = d.result.clone().filter(|_| d.text().is_some())
                {
                    match o {
                        dialogs::DiffOpt::Space => d.opts.ignore_space ^= true,
                        dialogs::DiffOpt::Case => d.opts.ignore_case ^= true,
                    }
                    return self.start_diff_with(Some(shown), None, None);
                }
            }
            Message::DiffCopy(to_right) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                    && let Some(shown) = d.result.clone()
                {
                    d.copying = Some((to_right, src));
                    d.confirm_close = false;
                    let block = d.block;
                    return self.start_diff_with(Some(shown), Some((block, to_right)), None);
                }
            }
            Message::DiffEdit(side) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                {
                    let text = if side == 0 { &src.0 } else { &src.1 };
                    let mut content = text_editor::Content::with_text(text);
                    // On the current block's line, where the change is.
                    let row = d.blocks().get(d.block).copied().unwrap_or(0);
                    let line = d
                        .text()
                        .map_or(0, |t| shagoff_core::diff::line_at(&t.rows, row, side));
                    // ponytail: the view follows the cursor when drawn, so the line shows at the
                    // bottom edge; centring it would need the editor's own scroll.
                    for _ in 0..line {
                        content.perform(text_editor::Action::Move(text_editor::Motion::Down));
                    }
                    d.editing = Some((side, content));
                    d.confirm_close = false;
                    return cosmic::iced::widget::operation::focus(d.edit_id.clone());
                }
            }
            Message::DiffEditAct(a) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some((_, content)) = &mut d.editing
                {
                    content.perform(a);
                }
            }
            Message::DiffEditDone(apply) => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some((side, content)) = d.editing.take()
                    && apply
                    && let Some(src) = d.text().map(|t| t.src.clone())
                    && let Some(texts) = shagoff_core::diff::edited(&src, side, &content.text())
                {
                    // As a copied block: undoable, that side unsaved (`DiffReady`).
                    d.copying = Some((side == 1, src));
                    return self.start_diff_with(None, None, Some(Arc::new(texts)));
                }
            }
            Message::DiffUndo => {
                // Not while a compare runs: it may hold a copy that would land after the undo.
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && d.text().is_some()
                    && let Some((src, dirty)) = d.undo.pop()
                {
                    d.dirty = dirty;
                    d.confirm_close = false;
                    return self.start_diff_with(None, None, Some(src));
                }
            }
            Message::DiffSave => {
                if let Some(Dialog::Diff(d)) = &mut self.dialog
                    && let Some(src) = d.text().map(|t| t.src.clone())
                {
                    let mut failed = None;
                    let stamps = d.stamps.map_or([None, None], |s| s.map(Some));
                    let mut new = d.stamps;
                    for (i, (dirty, path, text)) in [
                        (&mut d.dirty.0, &d.left, &src.0),
                        (&mut d.dirty.1, &d.right, &src.1),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if *dirty {
                            match shagoff_core::diff::save(path, text, stamps[i]) {
                                Ok(()) => {
                                    *dirty = false;
                                    // Saved by us: the next save checks against this.
                                    if let (Some(n), Ok(m)) = (&mut new, std::fs::metadata(path)) {
                                        n[i] = shagoff_core::diff::stamp_of(&m);
                                    }
                                }
                                Err(e) => failed = Some(format!("{}: {e}", path.display())),
                            }
                        }
                    }
                    d.stamps = new;
                    match failed {
                        // A side still unsaved: its undo history and the close warning stay.
                        Some(e) => self.say(StatusKind::Error, e),
                        None => {
                            d.confirm_close = false;
                            // The disk now holds what undoing would have changed.
                            d.undo.clear();
                            self.say(StatusKind::Info, fl!("diff-saved"));
                        }
                    }
                }
            }
            Message::DiffNext => return self.diff_step(1),
            Message::DiffPrev => return self.diff_step(-1),
            Message::DialogSubmit => return self.submit_dialog(),
            Message::DialogQueue => {
                let idle = self.job.is_none();
                let task = self.submit_dialog();
                if idle && let Some(j) = &mut self.job {
                    j.hidden = true;
                }
                return task;
            }
            Message::ListerLoaded(id, loaded) => {
                if let Some(l) = &mut self.lister
                    && l.id == id
                {
                    l.set_loaded(loaded);
                    if let Some(h) = l.raster_handle() {
                        l.wake_until = Some(std::time::Instant::now() + lister::WAKE_MAX);
                        return cosmic::iced::runtime::image::allocate(h).map(move |a| {
                            cosmic::Action::App(Message::ListerAllocated(id, a.ok()))
                        });
                    }
                }
            }
            Message::ListerPart(id, slide, loaded) => {
                let Some(l) = self.lister.as_deref_mut().filter(|l| l.id == id) else {
                    return Task::none();
                };
                if let Err(e) = loaded.as_ref() {
                    if l.slide == Some(slide) {
                        l.slide = None;
                        let e = e.clone();
                        self.say(StatusKind::Error, e);
                    }
                    return Task::none();
                }
                let part = l.set_window(slide, loaded);
                let q = l.query.clone();
                let scroll = part
                    .scroll
                    .map_or(Task::none(), |y| lister_scroll(l, l.offset.0, y));
                match part.found {
                    Some(true) => self.clear_busy(),
                    Some(false) => self.say(StatusKind::Info, fl!("lister-not-found", query = q)),
                    None => {}
                }
                let read = part.read.map_or(Task::none(), |s| self.lister_slide(s));
                return Task::batch([scroll, read]);
            }
            Message::ListerWake => {}
            Message::ListerAllocated(id, a) => {
                if let Some(l) = &mut self.lister
                    && l.id == id
                {
                    // Held in the main window's renderer; the viewer's own may need a frame or
                    // two more. Failed: that one loads it alone, the whole while.
                    if a.is_some() {
                        let soon = std::time::Instant::now() + lister::WAKE_AFTER;
                        l.wake_until = l.wake_until.map(|t| t.min(soon));
                    }
                    l.allocation = a;
                }
            }
            Message::ListerKey(k) if self.viewer() && self.dialog.is_none() && !self.busy() => {
                return match k {
                    ListerKey::Mode(m) => self.handle(Message::ListerMode(m)),
                    ListerKey::Next => self.lister_step(true),
                    ListerKey::Prev => self.lister_step(false),
                    ListerKey::Close => self.handle(Message::ListerClose),
                    ListerKey::Zoom(up) => self.lister_zoom(up),
                    ListerKey::FindPrev => self.lister_find(false, true),
                    ListerKey::Encoding(e) => self.handle(Message::ListerEncoding(e)),
                    ListerKey::Wrap => self.handle(Message::ListerWrap),
                };
            }
            Message::ListerKey(_) => {}
            Message::Letter(k, c) => {
                return self.handle(if self.viewer() {
                    Message::ListerKey(k)
                } else {
                    Message::CmdType(c)
                });
            }
            Message::ListerPointer(x, y) => {
                if let Some(l) = &mut self.lister {
                    l.pointer = Some((x, y));
                    if l.dragging
                        && let Some(at) = l.pointer_pos()
                    {
                        match &mut l.sel {
                            Some(s) => s.head = at,
                            // Pressed before the first move was known: it starts here.
                            None => l.sel = Some(shagoff_core::lister::Selection::new(at)),
                        }
                    }
                }
            }
            Message::ListerPointerLeft => {
                if let Some(l) = &mut self.lister {
                    l.pointer = None;
                }
            }
            Message::ListerDoubleClick => {
                if let Some(l) = &mut self.lister {
                    l.dragging = false; // this click's release
                    l.select_at_pointer(false);
                    let row = l.pointer_pos().map(|(r, _)| r);
                    l.double = row.map(|r| (std::time::Instant::now(), r));
                }
            }
            Message::ListerPress => {
                // A third click soon after a double one on the same row: the whole row.
                if let Some(l) = &mut self.lister
                    && let Some((t, row)) = l.double.take()
                    && t.elapsed() < std::time::Duration::from_millis(500)
                    && l.pointer_pos().is_some_and(|(r, _)| r == row)
                {
                    l.select_at_pointer(true);
                    return Task::none();
                }
                if let Some(l) = &mut self.lister
                    && l.mode != shagoff_core::lister::Mode::Image
                {
                    let at = l.pointer_pos();
                    match (&mut l.sel, at) {
                        (Some(s), Some(at)) if self.mods.shift() => s.head = at,
                        (_, at) => l.sel = at.map(shagoff_core::lister::Selection::new),
                    }
                    l.dragging = true;
                }
            }
            Message::ListerRelease => {
                if let Some(l) = &mut self.lister {
                    l.dragging = false;
                    if l.sel.is_some_and(|s| s.is_empty()) {
                        l.sel = None; // a plain click: nothing selected
                    }
                }
            }
            Message::ListerScrolled(x, y, h) => {
                if let Some(l) = &mut self.lister {
                    (l.offset, l.height) = ((x, y), h);
                    if let Some(s) = l.edge() {
                        return self.lister_slide(s);
                    }
                }
            }
            Message::ListerFit => {
                if let Some(l) = &mut self.lister {
                    l.zoom = None;
                }
            }
            Message::ListerResized(size) => {
                if let Some(l) = &mut self.lister {
                    l.resized(size.width, size.height);
                }
            }
            Message::ListerEncoding(e) => {
                if let Some(l) = &mut self.lister {
                    l.set_encoding(e);
                    // From hex it is now text: a hex offset points nowhere in it.
                    return lister_scroll(l, 0.0, 0.0);
                }
            }
            Message::ListerWrap => {
                if let Some(l) = &mut self.lister {
                    l.toggle_wrap();
                    // The rows changed: the old offset points into other text.
                    return lister_scroll(l, 0.0, 0.0);
                }
            }
            Message::ListerMode(m) => {
                if let Some(l) = &mut self.lister {
                    let before = l.mode;
                    l.set_mode(m);
                    if l.mode != before {
                        return lister_scroll(l, 0.0, 0.0);
                    }
                }
            }
            Message::ListerSearch => return self.lister_search(),
            Message::ListerQuery(q) => {
                if let Some(l) = &mut self.lister {
                    l.query = q;
                }
            }
            Message::ListerFind => {
                if let Some(l) = &mut self.lister {
                    l.searching = false;
                }
                return self.lister_find(true, false);
            }
            Message::ListerStep(forward) => return self.lister_step(forward),
            Message::ListerClose => {
                let window = self.lister.take().and_then(|l| l.window);
                if let Some(id) = window {
                    return cosmic::iced::window::close(id);
                }
            }
            Message::ListerWindow(w) => {
                if let Some(id) = self.lister.as_ref().and_then(|l| l.window) {
                    return match w {
                        ListerWin::Drag => cosmic::iced::window::drag(id),
                        ListerWin::Maximize => cosmic::iced::window::toggle_maximize(id),
                    };
                }
            }
            Message::WindowClosed(id) => {
                if self.lister.as_ref().is_some_and(|l| l.window == Some(id)) {
                    self.lister = None;
                }
            }
            Message::Linked(tab, made) => {
                let done = match made {
                    Ok((done, failed)) => {
                        if let Some(msg) = failed {
                            self.say(StatusKind::Error, msg);
                        }
                        done
                    }
                    Err(msg) => {
                        self.say(StatusKind::Error, msg);
                        return Task::none();
                    }
                };
                // Marks stay on the ones not linked, to try again. By id: the tab may have moved.
                if let Some(side) = self.side_of(tab)
                    && let Some(t) = (self.panes[side].items_mut().iter_mut()).find(|t| t.id == tab)
                {
                    t.panel.unmark(&done);
                }
                let reload = |app: &mut Self, s: usize| {
                    let to = app.panes[s].active().target();
                    app.reload(s, to, None)
                };
                return Task::batch([reload(self, 0), reload(self, 1)]);
            }
            Message::SideTreeRow(i) => {
                let active = self.active;
                self.click_pane(active);
                if let Some(st) = &mut self.side_tree {
                    st.tree.cursor = i.min(st.tree.rows.len().saturating_sub(1));
                    if let Some(n) = st.tree.current() {
                        let dir = n.path.clone();
                        // A locked tab there would open a tab at every click: it stays.
                        if self.panes[active].active().locked {
                            return Task::none();
                        }
                        st.synced = Some(dir.clone());
                        return self.load(active, dir, None);
                    }
                }
            }
            Message::SideTreeToggle(i) => {
                let hidden = self.panes[self.active].active().panel.show_hidden();
                if let Some(st) = &mut self.side_tree {
                    st.tree.cursor = i.min(st.tree.rows.len().saturating_sub(1));
                    if let Some(dir) = st.tree.toggle() {
                        return tree_read(st.id, vec![dir], hidden, None);
                    }
                }
            }
            Message::TreeRow(side, i) => {
                self.click_pane(side);
                if let Some(pt) = &mut self.panes[side].active_mut().tree {
                    pt.tree.cursor = i.min(pt.tree.rows.len().saturating_sub(1));
                    return Task::batch([self.reveal_active(side), self.tree_follow(side)]);
                }
            }
            Message::TreeRowToggle(side, i) => {
                self.click_pane(side);
                let hidden = self.panes[side].active().panel.show_hidden();
                if let Some(pt) = &mut self.panes[side].active_mut().tree {
                    pt.tree.cursor = i.min(pt.tree.rows.len().saturating_sub(1));
                    if let Some(dir) = pt.tree.toggle() {
                        return tree_read(pt.id, vec![dir], hidden, None);
                    }
                    // Closed: the cursor may be on the closed one now.
                    return Task::batch([self.reveal_active(side), self.tree_follow(side)]);
                }
            }
            Message::TreePick(i) => {
                if let Some(Dialog::Tree {
                    tree, hidden, id, ..
                }) = &mut self.dialog
                {
                    // On the row under the cursor: open / close it (Enter or «Go» goes there).
                    if tree.cursor == i {
                        if let Some(dir) = tree.toggle() {
                            return tree_read(*id, vec![dir], *hidden, None);
                        }
                        return Task::none();
                    }
                    tree.cursor = i.min(tree.rows.len().saturating_sub(1));
                }
            }
            Message::TreeFilled(from, read, select)
                if self.side_tree.as_ref().is_some_and(|s| s.id == from) =>
            {
                if let Some(st) = &mut self.side_tree {
                    for (dir, names) in read {
                        st.tree.fill(&dir, names);
                    }
                    if let Some(p) = &select {
                        st.tree.select(p);
                    }
                }
                // Only a read that put the cursor somewhere moves the view (not a toggle's).
                return match select {
                    Some(_) => self.side_tree_scroll(),
                    None => Task::none(),
                };
            }
            Message::TreeFilled(from, read, select) => {
                // A panel's tree (Ctrl+F8): filled, its cursor kept in view, the other panel
                // following it.
                let at = (0..2).find_map(|s| {
                    (self.panes[s].items().iter())
                        .position(|t| t.tree.as_ref().is_some_and(|pt| pt.id == from))
                        .map(|i| (s, i))
                });
                if let Some((side, i)) = at {
                    let t = &mut self.panes[side].items_mut()[i];
                    let tab = t.id;
                    if let Some(PanelTree { tree, .. }) = &mut t.tree {
                        for (dir, names) in read {
                            tree.fill(&dir, names);
                        }
                        if let Some(p) = &select {
                            tree.select(p);
                        }
                    }
                    let shown = self.panes[side].active().id == tab;
                    return match shown {
                        true => Task::batch([self.reveal(side, tab), self.tree_follow(side)]),
                        false => Task::none(),
                    };
                }
                if let Some(Dialog::Tree { tree, id, .. }) = &mut self.dialog
                    && *id == from
                {
                    for (dir, names) in read {
                        tree.fill(&dir, names);
                    }
                    if let Some(p) = select {
                        tree.select(&p);
                    }
                    return list_snap(tree.cursor, tree.rows.len());
                }
            }
            Message::Tested(results) => {
                self.testing = false;
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("testing-archives")) {
                    self.status = None;
                }
                if results.is_empty() {
                    return Task::none(); // stopped before the first one
                }
                let n = results.len();
                let bad: Vec<String> = (results.iter())
                    .filter_map(|(a, r)| {
                        let name = a.file_name().unwrap_or_default().to_string_lossy();
                        let e = r.as_ref().err()?;
                        Some(format!("{name}: {}", crate::why::message(e)))
                    })
                    .collect();
                if bad.is_empty() {
                    let files: u64 = results.iter().filter_map(|(_, r)| r.as_ref().ok()).sum();
                    self.say(StatusKind::Info, fl!("tested-ok", n = n, files = files));
                } else {
                    let head = fl!("tested-bad", bad = bad.len(), n = n);
                    self.say(StatusKind::Error, format!("{head}\n{}", bad.join("\n")));
                }
            }
            Message::TrashListed(side, r) => match r {
                Err(e) => self.say(StatusKind::Error, e),
                Ok(paths) if paths.is_empty() => self.say(StatusKind::Info, fl!("trash-empty")),
                Ok(paths) if self.dialog.is_none() && !self.busy() => {
                    self.dialog = Some(Dialog::ConfirmDelete {
                        side,
                        permanent: true,
                        paths,
                    });
                }
                Ok(_) => {}
            },
            Message::Restored(bad) => {
                if !bad.is_empty() {
                    self.say(StatusKind::Error, bad.join("\n"));
                }
                return Task::batch((0..2).map(|s| {
                    let to = self.panes[s].active().target();
                    self.reload(s, to, None)
                }));
            }
            Message::Summed(tab, r) => {
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("checksum-making")) {
                    self.status = None;
                }
                match r {
                    Ok(files) => {
                        let names: Vec<String> = (files.iter())
                            .map(|f| f.file_name().unwrap_or_default().to_string_lossy().into())
                            .collect();
                        let text = fl!("checksum-made", n = names.len(), names = names.join(", "));
                        self.say(StatusKind::Info, text);
                        // The cursor onto the (first) new file, the other panel just read again.
                        let reload = |app: &mut Self, s: usize, focus: Option<String>| {
                            let to = app.panes[s].active().target();
                            app.reload(s, to, focus)
                        };
                        let side = self.side_of(tab);
                        let focus = names.first().cloned();
                        return Task::batch([
                            reload(self, 0, focus.clone().filter(|_| side == Some(0))),
                            reload(self, 1, focus.filter(|_| side == Some(1))),
                        ]);
                    }
                    Err(e) if e.is_empty() => {} // stopped
                    Err(e) => self.say(StatusKind::Error, e),
                }
            }
            Message::Verified(results) => {
                self.testing = false;
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("checksum-verifying")) {
                    self.status = None;
                }
                if results.is_empty() {
                    return Task::none(); // stopped before the first one
                }
                let (mut ok, mut bad) = (0usize, Vec::new());
                for (sum, r) in &results {
                    let sum_name = sum.file_name().unwrap_or_default().to_string_lossy();
                    match r {
                        Err(e) => bad.push(format!("{sum_name}: {}", crate::why::message(e))),
                        Ok(rows) => {
                            for (name, c) in rows {
                                let name = name.display();
                                match c {
                                    Check::Ok => ok += 1,
                                    Check::NotFile => {
                                        bad.push(format!("{name}: {}", fl!("checksum-not-file")))
                                    }
                                    Check::Bad => {
                                        bad.push(format!("{name}: {}", fl!("checksum-bad")))
                                    }
                                    Check::Missing(k) => {
                                        let e = crate::why::text(&std::io::Error::from(*k));
                                        bad.push(format!("{name}: {e}"))
                                    }
                                }
                            }
                        }
                    }
                }
                if bad.is_empty() {
                    self.say(StatusKind::Info, fl!("checksum-ok", n = ok));
                } else {
                    let n = ok + bad.len();
                    let head = fl!("checksum-failed", bad = bad.len(), n = n);
                    self.say(StatusKind::Error, format!("{head}\n{}", bad.join("\n")));
                }
            }
            Message::Occupied(u) => {
                // Only its own "counting…": another job's Busy line stays.
                if (self.status.as_ref()).is_some_and(|s| s.text == fl!("counting")) {
                    self.status = None;
                }
                if let Some(u) = u {
                    let text = fl!(
                        "occupied",
                        size = crate::view::size_text(&self.config, u.bytes),
                        files = u.files,
                        dirs = u.dirs
                    );
                    self.say(StatusKind::Info, text);
                }
            }
            Message::TermWait => {} // `term_follow` runs after it
            Message::EditTick => {
                if self.dialog.is_some() || self.job.is_some() {
                    return Task::none(); // asked on a later tick
                }
                // Gone (temp cleaned, file deleted): stop watching it.
                self.edited
                    .retain(|e| std::fs::symlink_metadata(&e.file).is_ok());
                for e in &mut self.edited {
                    if let Ok(m) = std::fs::symlink_metadata(&e.file).and_then(|m| m.modified())
                        && m != e.mtime
                    {
                        e.mtime = m;
                        self.dialog = Some(Dialog::UpdateArchive {
                            file: e.file.clone(),
                            archive: e.archive.clone(),
                            entry: e.entry.clone(),
                        });
                        break;
                    }
                }
            }
            // Unsaved copied blocks: the first Esc / Cancel only warns.
            Message::DialogCancel
                if matches!(&self.dialog, Some(Dialog::Diff(d))
                    if (d.dirty.0 || d.dirty.1) && !d.confirm_close) =>
            {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    d.confirm_close = true;
                }
            }
            Message::Escape(window)
                if self
                    .lister
                    .as_ref()
                    .is_some_and(|l| l.window == Some(window)) =>
            {
                // A dialog or a job of the main window is answered first.
                if self.dialog.is_some() || self.busy() {
                    return self.handle(Message::DialogCancel);
                }
                return self.lister_escape();
            }
            Message::Escape(window) if window != self.window_id() => {
                return self.update(Message::Surface(cosmic::surface::action::destroy_popup(
                    window,
                )));
            }
            // The path line's field has the focus (it closes when it loses it).
            Message::Escape(_) if self.path_edit.is_some() && self.dialog.is_none() => {
                self.path_edit = None;
            }
            // Esc during a header-edge drag: back to the widths before it.
            Message::Escape(_) if self.col_drag.is_some() => {
                self.col_drag = None;
                self.cols = self.config.columns;
            }
            Message::Escape(_) if matches!(&self.dialog, Some(Dialog::Toolbar(t)) if t.picker.is_some()) =>
            {
                return self.update(Message::ToolEdit(crate::toolbar::ToolMsg::PickClose));
            }
            // A drag of ours: the compositor does not cancel it on Esc, we end it.
            Message::Escape(_) if self.dragging => {
                self.dragging = false;
                self.right_drag = None;
                self.drop_hover = None;
                return cosmic::iced::runtime::dnd::end_dnd();
            }
            // Esc in the compare's editor: out of editing, the dialog stays.
            Message::Escape(_) if matches!(&self.dialog, Some(Dialog::Diff(d)) if d.editing.is_some()) =>
            {
                return self.handle(Message::DiffEditDone(false));
            }
            Message::Escape(_) => return self.handle(Message::DialogCancel),
            Message::DialogCancel => match self.dialog.take() {
                Some(Dialog::Update {
                    stage: UpdateStage::Working,
                    ..
                }) => {
                    self.update_stop
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(Resolution::Cancel);
                }
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(ErrorChoice::Cancel);
                }
                Some(Dialog::Password { reply, .. }) => {
                    if let Some(r) = reply {
                        let _ = r.send(ErrorChoice::Cancel);
                    }
                }
                Some(Dialog::Connect { browsing, .. }) => {
                    if let Some(stop) = browsing {
                        stop.store(true, Ordering::Relaxed);
                    }
                    if let Some((_, cancel)) = &self.connecting {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Some(_) => {}
                None => {
                    if self.drawer.is_some() {
                        self.set_drawer(None);
                    } else if let Some(s) = self.search.take() {
                        if s.filter {
                            self.panes[s.side].active_mut().panel.set_filter(None);
                        }
                    } else if self.busy() {
                        self.cancel_job(); // not one in the background: Esc is for the panels
                    } else if self.viewer() {
                        return self.lister_escape();
                    } else if let Some((_, cancel)) = &self.connecting {
                        cancel.store(true, Ordering::Relaxed);
                    } else if (self.panes.iter())
                        .flat_map(|p| p.items())
                        .any(|t| t.branching.is_some())
                    {
                        // Every one under way: the hint does not say whose.
                        let mut back = Vec::new();
                        for side in 0..2 {
                            for (i, t) in self.panes[side].items_mut().iter_mut().enumerate() {
                                // Found, its rows being read: the dir as usual again.
                                if t.branching.is_some() && t.branch && t.results.is_some() {
                                    t.results = None;
                                    back.push((side, i, t.panel.cwd().to_path_buf()));
                                }
                                t.stop_branch();
                            }
                        }
                        self.say(StatusKind::Info, fl!("branch-stopped"));
                        let tasks: Vec<_> = (back.into_iter())
                            .map(|(side, i, cwd)| self.load_tab(side, i, cwd, None))
                            .collect();
                        return Task::batch(tasks);
                    } else if self.config.show_cmdline && !self.cmdline.is_empty() {
                        self.cmdline.clear();
                    } else {
                        self.panes[self.active].active_mut().panel.set_filter(None);
                    }
                }
            },
            Message::Op(event) => return self.on_job_event(event),
            Message::Pasted(Some((kind, paths))) if !paths.is_empty() && self.dialog.is_none() => {
                let op = match kind {
                    ClipKind::Copy => InputOp::Copy,
                    ClipKind::Cut => InputOp::Move,
                };
                // "": `cwd.join("")` ends with `/`, so `plan` always puts the files inside cwd.
                let before = (self.job.is_some(), self.queue.len());
                let task = self.start_transfer(op, self.active, paths, "");
                // Only when this paste started or queued a job: a refused one keeps the cut.
                if kind == ClipKind::Cut && (self.job.is_some(), self.queue.len()) != before {
                    return Task::batch([clip::clear(), task]);
                }
                return task;
            }
            Message::Pasted(_) => {}
            Message::MarksText(_, None) => self.say(StatusKind::Error, fl!("marks-no-text")),
            Message::MarksText(side, Some(text)) => {
                let n = self.panes[side].active_mut().panel.mark_listed(&text);
                self.say(StatusKind::Info, fl!("marks-loaded", count = n));
            }
            Message::DragStart(right) => {
                self.right_hold = None; // a drag, not a click: no mark, no menu
                self.right_drag = right;
                self.dragging = true;
            }
            Message::DragEnd => {
                self.dragging = false;
                self.drop_hover = None;
            }
            Message::DropAs(op) if matches!(self.dialog, Some(Dialog::DropMenu { .. })) => {
                if let Some(Dialog::DropMenu {
                    side, dir, sources, ..
                }) = self.dialog.take()
                {
                    self.dialog = Some(Dialog::Input {
                        op,
                        side,
                        sources,
                        input: dir_input(&dir),
                    });
                    return widget::text_input::focus(self.input_id.clone());
                }
            }
            Message::DropAs(_) => {} // the menu is gone already
            Message::DropRow(i) => {
                if let Some(Dialog::DropMenu { cursor, .. }) = &mut self.dialog {
                    *cursor = i;
                }
            }
            Message::DropAt(x, y) => {
                self.drop_at = Some(cosmic::iced::Point::new(x as f32, y as f32));
            }
            Message::DropHover(h) => self.drop_hover = h,
            Message::DropLeave(h) => {
                if self.drop_hover == Some(h) {
                    self.drop_hover = None;
                }
            }
            Message::Drop {
                side,
                dir,
                paths,
                mv,
            } => {
                self.drop_hover = None;
                // Any drop ends that drag, used or not: a later drop of the same files from
                // another program must not get the menu.
                let right_drag = self.right_drag.take();
                let at = self.drop_at.take().unwrap_or_default();
                let t = self.panes[side].active();
                // A job "In background" does not stop it: OK then says the job is running, as F5.
                // Search results have no one dir; a drop on a tab or a drive button names its own.
                if paths.is_empty()
                    || self.dialog.is_some()
                    || (dir.is_none() && t.results.is_some())
                {
                    return Task::none();
                }
                let dir = dir.unwrap_or_else(|| t.panel.cwd().to_path_buf());
                if shagoff_core::panel::drop_is_noop(&paths, &dir) {
                    return Task::none();
                }
                // The pane the files came from, if one shows them: its marks go when done, as F5.
                let parent = paths.first().and_then(|p| p.parent());
                let from = (0..2).find(|&s| Some(self.panes[s].active().panel.cwd()) == parent);
                let side = from.unwrap_or(side);
                // Our own right-button drag (the same files): ask, as TC's drop menu.
                if right_drag.is_some_and(|r| *r == paths) {
                    self.dialog = Some(Dialog::DropMenu {
                        side,
                        dir,
                        sources: paths,
                        at,
                        cursor: 0,
                    });
                    // A focused command line would take the menu's letters and Enter.
                    return unfocus();
                }
                // Shift as we see it: the compositor's chosen action ignores it (COSMIC keeps the
                // destination's preferred Copy), but the window keeps the keyboard while dragging.
                let mv = mv || self.mods.shift();
                // As F5 / F6 in TC: the same dialog, aimed at the dir the files were dropped on.
                self.dialog = Some(Dialog::Input {
                    op: if mv { InputOp::Move } else { InputOp::Copy },
                    side,
                    sources: paths,
                    input: dir_input(&dir),
                });
                return widget::text_input::focus(self.input_id.clone());
            }
            Message::Resolve(r) => match self.dialog.take() {
                Some(Dialog::Conflict { reply, .. }) => {
                    let _ = reply.send(r);
                }
                other => self.dialog = other,
            },
            Message::ErrorAnswer(c) => match self.dialog.take() {
                Some(Dialog::Error { reply, .. }) => {
                    let _ = reply.send(c);
                }
                other => self.dialog = other,
            },
            Message::CancelJob => self.cancel_job(),
            Message::JobPause => {
                if let Some(j) = &self.job {
                    j.control.pause.fetch_xor(true, Ordering::Relaxed);
                }
            }
            Message::QueueUp(i) => {
                if (1..self.queue.len()).contains(&i) {
                    self.queue.swap(i - 1, i);
                }
            }
            Message::Unqueue(i) => {
                if i < self.queue.len() {
                    self.queue.remove(i);
                    // The «В очереди: N» said when it was added is stale now.
                    self.status = None;
                }
            }
            Message::JobHide | Message::JobShow => {
                if let Some(j) = &mut self.job {
                    j.hidden = matches!(message, Message::JobHide);
                }
            }
            Message::SearchInput(text) => {
                let Some(s) = &self.search else {
                    return Task::none();
                };
                let (side, filter) = (s.side, s.filter);
                let t = self.panes[side].active_mut();
                if filter {
                    t.panel.set_filter(Some(text.clone()));
                } else if !text.is_empty() {
                    match t.panel.find(&text, 0, true) {
                        Some(i) => t.panel.set_cursor(i),
                        None => return Task::none(), // rejected: the field keeps the old text
                    }
                }
                let tab = t.id;
                if let Some(s) = &mut self.search {
                    s.text = text;
                }
                return self.reveal(side, tab);
            }
            // Only the quick search field forwards keys; a dialog's text field keeps its own.
            Message::ListPick(i) => return self.pick(i),
            Message::ListClear => match self.dialog.take() {
                Some(Dialog::List {
                    kind: ListKind::History,
                    side,
                    ..
                }) => self.panes[side].active_mut().history.clear(),
                Some(Dialog::List {
                    kind: ListKind::Commands,
                    ..
                }) => self.commands.clear(),
                other => self.dialog = other,
            },
            Message::OpenWithApps(side, files, r) => match r {
                // Asked in the background: a dialog opened meanwhile keeps its place.
                Ok(_) if self.dialog.is_some() => {}
                Ok(found) => {
                    self.open_with = files;
                    self.open_with_mime = found.mime;
                    self.dialog = Some(Dialog::List {
                        kind: ListKind::OpenWith,
                        side,
                        cursor: 0,
                        items: open_with_items(found.apps, false),
                    });
                }
                Err(e) => self.say(StatusKind::Error, fl!("open-failed", err = e)),
            },
            // "All programs…": the list shown is still the one that asked.
            Message::OpenWithAll(apps) => {
                if let Some(Dialog::List {
                    kind: ListKind::OpenWith,
                    cursor,
                    items,
                    ..
                }) = &mut self.dialog
                {
                    *items = open_with_items(apps, true);
                    *cursor = 0;
                    return scrollable::snap_to(
                        dialogs::list_scroll_id(),
                        scrollable::RelativeOffset {
                            x: None,
                            y: Some(0.0),
                        },
                    );
                }
            }
            Message::BranchListed {
                tab,
                id,
                root,
                paths,
            } => {
                // By id: the tab may have moved to the other panel meanwhile (Ctrl+U, a drag).
                let Some(side) = self.side_of(tab) else {
                    return Task::none();
                };
                let Some(i) = (self.panes[side].items().iter()).position(|t| t.id == tab) else {
                    return Task::none();
                };
                let t = &mut self.panes[side].items_mut()[i];
                // The tab went somewhere meanwhile (even back here), or asked again: not wanted
                // (and the status is someone else's by now).
                if t.branching != Some(id) {
                    return Task::none();
                }
                let t = &mut self.panes[side].items_mut()[i];
                if let Some(paths) = paths {
                    t.results = Some((root.clone(), Arc::new(paths)));
                    t.branch = true;
                    // The rows are read next, still under way (`Listed` ends it).
                    if self.panes[side].active().id == tab {
                        return self.reload(side, root, None);
                    }
                }
                self.panes[side].items_mut()[i].branching = None; // stopped, or read when shown
                if (self.status.as_ref()).is_some_and(|s| s.kind == StatusKind::Busy) {
                    self.status = None;
                }
            }
            Message::OpenWithDefault => {
                let Some(Dialog::List {
                    kind: ListKind::OpenWith,
                    cursor,
                    items,
                    ..
                }) = &self.dialog
                else {
                    return Task::none();
                };
                let Some(item) = items.get(*cursor).filter(|it| it.kind == Item::App) else {
                    return Task::none();
                };
                let (name, desktop, mime) = (
                    item.label.clone(),
                    item.path.clone(),
                    self.open_with_mime.clone(),
                );
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            openwith::set_default(&mime, &desktop).map(|()| name)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::OpenWithDefaultSet(r)),
                );
            }
            Message::OpenWithDefaultSet(r) => match r {
                Ok(app) => {
                    let mime = self.open_with_mime.clone();
                    self.say(
                        StatusKind::Info,
                        fl!("open-with-default-set", app = app, mime = mime),
                    );
                }
                Err(e) => self.say(StatusKind::Error, fl!("open-failed", err = e)),
            },
            // A Shift-press moves the button, it doesn't run it.
            Message::Tool(_) if self.mods.shift() || self.tool_drag.grab.is_some() => {}
            Message::Tool(i) => return self.run_tool(i),
            Message::ToolHover(at) => {
                self.tool_drag.hover(at, &mut self.config.toolbar);
            }
            Message::ToolLeft(i) => self.tool_drag.left(i),
            // Under a dialog the last button entered may still count as hovered.
            Message::ToolGrab if self.dialog.is_none() => self.tool_drag.grab(),
            Message::ToolGrab => {}
            Message::ToolRelease => self.tool_drop(),
            Message::ToolFailed(e) => self.say(StatusKind::Error, fl!("cmd-failed", error = e)),
            Message::Colors(m) => {
                if let Some(Dialog::Colors(c)) = &mut self.dialog {
                    c.update(m);
                }
            }
            Message::FontFilter(s) => {
                if let Some(Drawer::Settings(f)) = &mut self.drawer {
                    f.font_filter = s;
                }
            }
            Message::KeyEdit(m) => {
                if let Some(Dialog::Keys(k)) = &mut self.dialog {
                    k.update(m);
                }
            }
            Message::ToolEdit(m) => {
                if let Some(Dialog::Toolbar(t)) = &mut self.dialog {
                    let open = matches!(m, crate::toolbar::ToolMsg::PickOpen);
                    t.update(m);
                    if open && t.picker.is_some() {
                        return Task::perform(
                            async {
                                tokio::task::spawn_blocking(crate::toolbar::icon_names)
                                    .await
                                    .unwrap_or_default()
                            },
                            |v| {
                                cosmic::Action::App(Message::ToolEdit(
                                    crate::toolbar::ToolMsg::PickLoaded(v),
                                ))
                            },
                        );
                    }
                }
            }
            Message::Hot(m) => {
                if let Some(Dialog::Hotlist(h)) = &mut self.dialog {
                    let cwd = self.panes[h.side].active().panel.cwd().to_path_buf();
                    h.update(m, &cwd);
                }
            }
            // Quick search, viewer search or command line; not a dialog's or a settings field.
            // F2 only: Shift+F6 is Rename too.
            Message::FieldKey(Action::Rename)
                if !self.mods.shift()
                    && matches!(
                        self.dialog,
                        Some(Dialog::Input {
                            op: InputOp::Copy | InputOp::Move,
                            ..
                        })
                    ) =>
            {
                return self.handle(Message::DialogQueue);
            }
            // Ctrl+X passed on by a field. In the command line, on a line from the history
            // (Ctrl+E): the next command (TC). A typed line stays (the field cut its selection);
            // the other fields, the menu's «Cut» and the panel's Ctrl+X never step the history.
            Message::FieldKey(Action::ClipCut) => {
                if self.cmd_ready()
                    && self.commands.contains(&self.cmdline)
                    && let Some(next) = cmdline::next(&self.commands, &self.cmdline)
                {
                    self.cmdline = next;
                    return widget::text_input::move_cursor_to_end(self.cmd_id.clone());
                }
            }
            Message::FieldKey(action) => {
                if let Some(task) = self.mask_history(action) {
                    return task;
                }
                if self.dialog.is_none() && self.drawer.is_none() {
                    return self.handle(Message::Key(action));
                }
                // Tab is ours (pane switch), so the settings fields get it here: next field.
                if action == Action::SwitchPane && self.dialog.is_none() {
                    return cosmic::iced::runtime::widget::operation::focus_next();
                }
            }
            Message::CmdInput(text) => {
                self.cmdline = text;
                // Erased: the panel gets its keys back (Backspace goes up again).
                if self.cmdline.is_empty() {
                    return unfocus();
                }
            }
            // The field's Enter comes before the modifier change of the same key batch: wait a
            // step, or Shift+Enter ran without the terminal (the field's message is handled first).
            Message::CmdSubmit => return cosmic::task::message(Message::CmdEnter),
            // The field takes Ctrl+Enter as Enter: here it inserts the name / path under the cursor.
            Message::CmdEnter if self.mods.control() => {
                let action = if self.mods.shift() {
                    Action::CmdPath
                } else {
                    Action::CmdName
                };
                return self
                    .cmd_action(self.active, action)
                    .unwrap_or_else(Task::none);
            }
            Message::CmdEnter => return self.cmd_run(self.mods.shift()),
            Message::CmdType(c) => {
                // The drop menu's hot letters (`&` in its items' names) pick at once.
                if let Some(Dialog::DropMenu { sources, .. }) = &self.dialog {
                    use shagoff_core::quicksearch::hotkey;
                    let key = c.to_lowercase().next();
                    let op = (dialogs::drop_ops(sources).iter().copied())
                        .find(|&op| hotkey(&dialogs::drop_label(op)).1.map(|(_, k)| k) == key);
                    return op.map_or_else(Task::none, |op| self.handle(Message::DropAs(op)));
                }
                // The tree: the next dir with that letter, as in a list.
                if let Some(Dialog::Tree { tree, .. }) = &mut self.dialog {
                    let names = tree.rows.iter().map(|n| n.name.as_str());
                    if let Some(i) = shagoff_core::quicksearch::next_with(names, tree.cursor, c) {
                        tree.cursor = i;
                        return list_snap(i, tree.rows.len());
                    }
                    return Task::none();
                }
                // A list dialog's letter key, as in TC's menus: the next item with that letter.
                if let Some(Dialog::List {
                    kind,
                    cursor,
                    items,
                    ..
                }) = &mut self.dialog
                {
                    use shagoff_core::quicksearch::{hotkey, next_with};
                    // Ctrl+D: a `&` hot letter picks its item at once; shared by several, it
                    // steps through them.
                    let hot: Vec<usize> = (items.iter().enumerate())
                        .filter(|(_, it)| {
                            *kind == ListKind::Hotlist
                                && hotkey(&it.label).1.map(|(_, k)| k) == c.to_lowercase().next()
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let [i] = hot[..] {
                        return self.pick(i);
                    }
                    if let Some(&i) = hot.iter().find(|&&i| i > *cursor).or(hot.first()) {
                        *cursor = i;
                        return list_snap(i, items.len());
                    }
                    let labels = items.iter().map(|it| it.label.as_str());
                    if let Some(i) = next_with(labels, *cursor, c) {
                        *cursor = i;
                        return list_snap(i, items.len());
                    }
                    return Task::none();
                }
                if self.cmd_ready() {
                    self.cmdline.push(c);
                    return self.cmd_focus();
                }
            }
            Message::SearchSubmit => {
                if let Some(s) = self.search.take()
                    && !s.filter
                {
                    return self.act(s.side, Action::Enter);
                }
            }
            Message::Changed(side) => {
                let t = self.panes[side].active();
                // A change in the dir we are leaving must not cancel the scan of the one we enter.
                // Search results / a branch: not read again on every event (all of them stat'ed,
                // a million for a big tree; TC neither) — Ctrl+R does it.
                if self.job.is_none() && t.target() == t.panel.cwd() && t.results.is_none() {
                    let cwd = t.target();
                    return self.reload(side, cwd, None);
                }
            }
            Message::WindowWidth(window, w) if window == self.window_id() => self.win_w = w,
            Message::WindowWidth(..) => {}
            Message::Dismiss(None) => self.status = None,
            Message::Dismiss(Some(side)) => self.panes[side].active_mut().error = None,
            Message::SplitStart => {
                (self.splitting, self.split_from, self.split_y0) = (true, None, None)
            }
            // Stacked panes: the divider follows the pointer's height, from where it was grabbed.
            Message::SplitAt(window, _, y)
                if self.splitting && self.vertical && !self.wide && window == self.window_id() =>
            {
                let (y0, r0) = *self
                    .split_y0
                    .get_or_insert((y, viewport::split_or_half(self.split)));
                if (y - y0).abs() >= 3.0 {
                    self.split_from = Some(f32::NAN); // moved for real: follow from now on
                }
                if self.split_from.is_some_and(f32::is_nan) {
                    // The lists plus what each pane has above its list (drives, path, header).
                    let total =
                        self.panes[0].active().height + self.panes[1].active().height + 160.0;
                    self.split = viewport::split_ratio_by(r0, y - y0, total);
                }
            }
            Message::SplitAt(window, x, _) if self.splitting && window == self.window_id() => {
                let from = *self.split_from.get_or_insert(x);
                if (x - from).abs() >= 3.0 {
                    self.split_from = Some(f32::NAN); // moved for real: follow from now on
                }
                if self.split_from.is_some_and(f32::is_nan) {
                    let (pad, gap) = crate::view::split_geometry(self.config.skin);
                    // Before the window's width is known: the two lists' (no Modern cards).
                    let total = match self.win_w {
                        w if w > 0.0 => w - 2.0 * pad - gap,
                        _ => self.panes[0].active().width + self.panes[1].active().width,
                    };
                    self.split = viewport::split_ratio(pad + gap / 2.0, total, x);
                }
            }
            Message::SplitAt(..) => {}
            Message::SplitEnd => self.splitting = false,
            Message::SplitReset => {
                (self.split, self.splitting) = (0.5, false);
            }
            Message::ColDragStart(side, edge) => self.col_drag = Some((side, edge, None)),
            Message::ColDrag(x) => {
                if let Some((_, edge, start)) = &mut self.col_drag {
                    // The edge's column in the set shown; one with no stored width stays.
                    let slot = shagoff_core::columns::parse(&self.config.col_set)
                        .get(viewport::drag_target(*edge))
                        .and_then(|c| c.slot());
                    if let Some(slot) = slot {
                        // On screen the widths are scaled with the text (`col_widths`); kept unscaled.
                        let scale = self.config.text_scale();
                        let &mut (x0, w0) = start.get_or_insert((x, self.cols[slot] * scale));
                        let (_, w) = viewport::drag_col(*edge, w0, x0, x);
                        self.cols[slot] = w / scale;
                    }
                }
            }
            // The main window's only (a context menu popup has its own coordinates).
            Message::TabDragAt(window, x) if window == self.window_id() => {
                let split = crate::view::pane_split(
                    self.config.skin,
                    self.panes[0].active().width,
                    self.config.active_frame,
                );
                // «Panel at 100 %»: the one pane shown is the only one to hold it over.
                // Stacked panes: x says nothing, a tab is not dropped on the other pane.
                self.pane_hover = match (self.wide, self.vertical) {
                    (true, _) => Some(self.active),
                    (_, true) => None,
                    _ => Some(usize::from(x > split)),
                };
            }
            Message::TabDragAt(..) => {}
            Message::ColDragEnd => {
                if self.col_drag.take().is_some() {
                    self.save_columns();
                }
                // A tab held over the other pane: it goes there (TC), with Ctrl a copy. Not
                // while the Ctrl+Q viewer covers that pane.
                let over = self.pane_hover.take();
                let shown = self.viewer_shown();
                let covered = |to: usize| match shown {
                    None => false,
                    Some(None) => true,
                    Some(Some(s)) => s == to,
                };
                if let Some((from, _)) = self.tab_drag.take()
                    && over == Some(1 - from)
                    && !covered(1 - from)
                {
                    let action = match self.mods.control() {
                        true => Action::TabCopyOther,
                        false => Action::TabMoveOther,
                    };
                    return self.act(from, action);
                }
            }
            Message::PathEditStart(side) => return self.edit_path(side),
            Message::PathEdit(s) => {
                if let Some((_, text)) = &mut self.path_edit {
                    *text = s;
                }
            }
            Message::PathEditGo => {
                let Some((side, text)) = self.path_edit.take() else {
                    return Task::none();
                };
                if self.busy() {
                    return Task::none();
                }
                let home = std::env::home_dir().unwrap_or_else(|| "/".into());
                let cwd = self.shown(self.panes[side].active().panel.cwd());
                let path = self.real(&cmdline::resolve(&text, &cwd, &home));
                // Off the UI thread: a dead network mount would hang a stat.
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let focus = file_in_its_dir(&path);
                            (path, focus)
                        })
                        .await
                    },
                    move |r| match r {
                        Ok((path, focus)) => {
                            cosmic::Action::App(Message::PathEditFound(side, path, focus))
                        }
                        Err(_) => cosmic::Action::None,
                    },
                );
            }
            Message::PathEditCancel => self.path_edit = None,
            Message::PathEditFound(side, path, focus) => {
                // A file: its dir, the cursor on it (an archive is entered, as Enter does).
                return match (focus, path.parent()) {
                    (Some(name), Some(dir)) => self.load(side, dir.to_path_buf(), Some(name)),
                    _ => self.load(side, path, None),
                };
            }
            Message::PathPart(side, path) => {
                self.search = None;
                if self.busy() {
                    return Task::none();
                }
                let focus = panel::child_toward(self.panes[side].active().panel.cwd(), &path);
                self.active = side;
                return self.load(side, path, focus);
            }
            Message::Drive(side, path) => {
                self.search = None;
                if !self.busy() {
                    self.dialog = None;
                    return self.go_to(side, path);
                }
            }
            Message::ConnectPassword(s) => {
                if let Some(Dialog::Connect { password, .. }) = &mut self.dialog {
                    *password = s;
                }
            }
            Message::ConnectKeep(on) => {
                if let Some(Dialog::Connect { keep, .. }) = &mut self.dialog {
                    *keep = on;
                }
            }
            Message::Kept(r) => {
                if let Err(mount::Error::Failed(e)) = r {
                    self.say(StatusKind::Error, fl!("keyring-failed", error = e));
                }
            }
            Message::AllVolumes(vols) => {
                self.volumes = vols.into_iter().filter(|v| v.mount.is_none()).collect();
            }
            Message::MountVolume(side, device) => {
                self.search = None;
                if self.busy() {
                    return Task::none();
                }
                self.dialog = None;
                self.say(StatusKind::Busy, fl!("mounting"));
                return blocking(
                    move || mount::mount_device(&device),
                    move |r| Message::Mounted(side, r),
                );
            }
            Message::Volumes(side, vols) => {
                if let Some(Dialog::List {
                    kind: ListKind::Drives,
                    side: s,
                    items,
                    ..
                }) = &mut self.dialog
                    && *s == side
                {
                    let fresh: Vec<_> = vols
                        .into_iter()
                        .filter(|v| v.mount.is_none())
                        .filter(|v| {
                            !items
                                .iter()
                                .any(|i| i.path.as_os_str() == v.device.as_str())
                        })
                        .collect();
                    items.extend(fresh.into_iter().map(|v| ListItem {
                        label: v.name,
                        path: v.device.into(),
                        kind: Item::Mount,
                        icon: String::new(),
                    }));
                }
            }
            Message::SplitSize(s) => {
                if let Some(Dialog::Split { size, .. }) = &mut self.dialog {
                    *size = s;
                }
            }
            Message::Pieces(done) => {
                self.clear_busy();
                match done {
                    Ok(note) if note.is_empty() => {}
                    Ok(note) => self.say(StatusKind::Info, note),
                    Err(e) => self.say(StatusKind::Error, e),
                }
                // The parts / the joined file show up where they went.
                let tasks: Vec<_> = (0..2)
                    .map(|s| {
                        let to = self.panes[s].active().target();
                        self.reload(s, to, None)
                    })
                    .collect();
                return Task::batch(tasks);
            }
            Message::Made {
                side,
                cwd,
                name,
                first,
                what,
                made,
            } => match made {
                Ok(()) => {
                    if what == New::File {
                        let argv = launch::command(
                            &self.config.editor,
                            &["cosmic-edit"],
                            &cwd.join(&name),
                        );
                        if let Err(err) = spawn_detached(&argv) {
                            self.say(
                                StatusKind::Error,
                                fl!("open-failed", err = crate::why::text(&err)),
                            );
                        }
                    }
                    // Still that dir: the cursor goes onto the new one.
                    if self.panes[side].active().target() == cwd {
                        return match what {
                            // `load`: out of search results too.
                            New::Copy => self.load(side, cwd, first),
                            _ => self.reload(side, cwd, first),
                        };
                    }
                }
                Err(e) if what == New::Dir => {
                    self.say(StatusKind::Error, fl!("mkdir-failed", path = name, err = e))
                }
                Err(e) => self.say(
                    StatusKind::Error,
                    fl!("new-file-failed", path = name, err = e),
                ),
            },
            Message::Terminals(list, default) => {
                (self.terminals, self.term_default) = (list, default)
            }
            Message::TermTx(tx) => {
                self.term_tx = Some(tx);
                self.term_start();
            }
            Message::TermStart => {
                self.term_ended = false;
                self.term_start();
            }
            Message::TermDragStart => {
                self.term_rows = self.config.term_rows; // a press let go unmoved saves the same
                self.term_drag = Some((f32::NAN, self.term_rows));
            }
            Message::TermDragAt(y) => {
                if let Some((from, rows)) = &mut self.term_drag {
                    if from.is_nan() {
                        *from = y;
                    }
                    let line = (f32::from(self.config.panel_size()) * 1.4).ceil();
                    let n = f32::from(*rows) + (*from - y) / line;
                    self.term_rows = n.round().clamp(3.0, 60.0) as u16;
                }
            }
            Message::TermDragEnd => {
                if self.term_drag.take().is_some() {
                    return self.set(Setting::TermRows(self.term_rows));
                }
            }
            Message::TermEvent(e) => {
                use crate::term::Outcome;
                let Some(pane) = &self.term else {
                    return Task::none();
                };
                match crate::term::on_event(pane, e) {
                    Outcome::None => {}
                    Outcome::Exit => {
                        (self.term, self.term_full, self.term_ended) = (None, false, true);
                    }
                    Outcome::Copy(text) => return cosmic::iced::clipboard::write(text),
                }
            }
            Message::Gvfs(drives) => {
                self.gvfs_busy = false;
                if drives != self.gvfs {
                    self.gvfs = drives;
                    self.set_drives();
                }
            }
            Message::Mounted(side, result) => {
                self.refresh_mounts();
                let volumes = self.refresh_volumes(true);
                match result {
                    Ok(path) => {
                        self.clear_busy();
                        return Task::batch([self.go_to(side, path), volumes]);
                    }
                    Err(mount::Error::Cancelled) => {
                        self.status = None; // the Busy "Connecting…"
                        self.say(StatusKind::Info, fl!("connect-cancelled"));
                    }
                    Err(e) => self.say(StatusKind::Error, mount_error(&e)),
                }
                return volumes;
            }
            // Ctrl+F's result: remembered on success, then as any mount.
            Message::Connected(side, url, result) => {
                if self.connecting.as_ref().is_some_and(|(u, _)| *u == url) {
                    self.connecting = None;
                }
                if result.is_ok() {
                    let list = mount::remember(&self.config.connections, &url);
                    self.save_connections(list);
                    let mut keep = Task::none();
                    if let Some(Dialog::Connect {
                        browsing,
                        password,
                        keep: on,
                        ..
                    }) = &self.dialog
                    {
                        if let Some(stop) = browsing {
                            stop.store(true, Ordering::Relaxed); // a browse no one will see
                        }
                        if *on && !password.is_empty() {
                            let (url, password) = (url.clone(), password.clone());
                            let no_realm = fl!("keyring-no-realm");
                            keep = blocking(
                                move || {
                                    use shagoff_core::keyring::{self, KeepError};
                                    keyring::save(&url, &password).map_err(|e| {
                                        mount::Error::Failed(match e {
                                            KeepError::NoRealm => no_realm,
                                            KeepError::Failed(e) => e,
                                        })
                                    })
                                },
                                Message::Kept,
                            );
                        }
                        self.dialog = None;
                    }
                    return Task::batch([keep, self.handle(Message::Mounted(side, result))]);
                } else if let Some(Dialog::Connect { note, .. }) = &mut self.dialog
                    && let Err(e) = &result
                    && *e != mount::Error::Cancelled
                {
                    *note = Some((true, mount_error(e)));
                    self.status = None; // the Busy "Connecting…"
                    return Task::none();
                }
                return self.handle(Message::Mounted(side, result));
            }
            Message::ConnectPick(u) => {
                if let Some(Dialog::Connect { url, .. }) = &mut self.dialog {
                    *url = u;
                }
                return widget::text_input::focus(self.input_id.clone());
            }
            // By value: the config may have changed since the dialog opened.
            Message::ConnectForget(url) => {
                let mut list = self.config.connections.clone();
                list.retain(|u| *u != url);
                if let Some(Dialog::Connect { saved, .. }) = &mut self.dialog {
                    saved.clone_from(&list);
                }
                self.save_connections(list);
            }
            Message::ConnectBrowse(place) => {
                if let Some(Dialog::Connect {
                    url,
                    password,
                    found,
                    browsing,
                    at,
                    note,
                    ..
                }) = &mut self.dialog
                {
                    if let Some(stop) = browsing.take() {
                        stop.store(true, Ordering::Relaxed);
                        // The button again while looking: only stop.
                        if place.is_none() {
                            return Task::none();
                        }
                    }
                    *at = place.unwrap_or_else(|| mount::NETWORK.to_string());
                    found.clear();
                    *note = None;
                    let stop = Arc::new(AtomicBool::new(false));
                    *browsing = Some(stop.clone());
                    // The password was typed for the address's host: no one else gets it.
                    let password = if mount::host(url) == mount::host(at) {
                        password.clone()
                    } else {
                        String::new()
                    };
                    let (place, mine) = (at.clone(), stop.clone());
                    return blocking(
                        move || mount::browse(&place, &password, &stop),
                        move |r| Message::Browsed(mine, r),
                    );
                }
            }
            Message::Browsed(stop, r) => {
                if let Some(Dialog::Connect {
                    found,
                    browsing,
                    note,
                    ..
                }) = &mut self.dialog
                {
                    // Stopped or replaced by another: not wanted.
                    if !browsing.as_ref().is_some_and(|b| Arc::ptr_eq(b, &stop)) {
                        return Task::none();
                    }
                    *browsing = None;
                    *note = match r {
                        Ok(list) if list.is_empty() => Some((false, fl!("connect-none-found"))),
                        Ok(list) => {
                            *found = list;
                            None
                        }
                        Err(e) => Some((true, mount_error(&e))),
                    };
                }
            }
            Message::Unmounted(root, result) => {
                self.refresh_mounts();
                self.volumes_at = None; // the next listing asks gio again
                if let Err(e) = result {
                    self.say(StatusKind::Error, mount_error(&e));
                    return Task::none();
                }
                self.clear_busy();
                // ponytail: only the active tab of each pane moves; other tabs inside show an error.
                let inside: Vec<usize> = (0..2)
                    .filter(|&s| self.panes[s].active().target().starts_with(&root))
                    .collect();
                let home = self.home.clone();
                return Task::batch(inside.into_iter().map(|s| self.load(s, home.clone(), None)));
            }
            Message::Config(c) => return self.apply_config(*c),
            Message::CloseDrawer => self.set_drawer(None),
            Message::FocusNext => return cosmic::iced::widget::operation::focus_next(),
            Message::Setting(s) => return self.set(s),
            Message::CheckUpdate => {
                use shagoff_core::update::{Status, check};
                self.update_check = Some(Status::Checking);
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| check(env!("CARGO_PKG_VERSION")))
                            .await
                            .unwrap_or_else(|e| Status::Failed(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::UpdateChecked(r)),
                );
            }
            Message::UpdateChecked(shagoff_core::update::Status::Available(tag)) => {
                // Asked by hand: the update window, whatever was skipped before.
                self.update_check = None;
                self.set_drawer(None);
                self.dialog = Some(Dialog::Update {
                    tag,
                    stage: UpdateStage::Ask,
                });
            }
            Message::UpdateChecked(s) => self.update_check = Some(s),
            Message::StartupUpdate(s) => {
                // A failed check is not worth a word at start (no network).
                if let shagoff_core::update::Status::Available(tag) = s
                    && tag != self.update_skip
                    && self.dialog.is_none()
                {
                    self.dialog = Some(Dialog::Update {
                        tag,
                        stage: UpdateStage::Ask,
                    });
                }
            }
            Message::UpdateSkip => {
                if let Some(Dialog::Update { tag, .. }) = self.dialog.take() {
                    self.update_skip = tag;
                }
            }
            Message::UpdateNow => {
                let Some(Dialog::Update { tag, stage }) = &mut self.dialog else {
                    return Task::none();
                };
                if matches!(stage, UpdateStage::Working) {
                    return Task::none(); // a second press must not start a second install
                }
                *stage = UpdateStage::Working;
                let tag = tag.clone();
                self.update_stop = Default::default();
                let stop = self.update_stop.clone();
                if !shagoff_core::update::can_install() {
                    *stage = UpdateStage::Failed(fl!("update-manual"));
                    return Task::none();
                }
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            // A fresh dir of ours in ~/.cache (not a shared /tmp), swept at the
                            // next start if this run dies.
                            let dir = archive::fresh_temp_dir(&archive::temp_root())
                                .map_err(|e| e.to_string())?;
                            let r =
                                shagoff_core::update::download(&tag, &dir).and_then(
                                    |deb| match stop.load(std::sync::atomic::Ordering::Relaxed) {
                                        true => Err("cancelled".into()),
                                        false => shagoff_core::update::install(&deb),
                                    },
                                );
                            let _ = std::fs::remove_dir_all(&dir);
                            r
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| cosmic::Action::App(Message::UpdateDone(r)),
                );
            }
            Message::UpdateDone(r) => {
                if let Some(Dialog::Update { stage, .. }) = &mut self.dialog {
                    *stage = match r {
                        Ok(()) => UpdateStage::Done,
                        Err(e) => UpdateStage::Failed(e),
                    };
                }
            }
            Message::OpenUrl(url) => {
                if let Err(e) = spawn_detached(&["xdg-open".into(), url.into()]) {
                    log::warn!("xdg-open: {e}");
                }
            }
            Message::CloseRequested(id) if id == self.window_id() => {
                return self.handle(Message::Quit);
            }
            Message::CloseRequested(_) => {}
            // Confirmed, but the worker hangs in blocking I/O (a dead network mount): × again quits.
            Message::Quit if self.quitting => return self.handle(Message::Exit),
            // Unsaved copied blocks in the comparison: the first × only warns, as Esc.
            Message::Quit
                if matches!(&self.dialog, Some(Dialog::Diff(d))
                    if (d.dirty.0 || d.dirty.1) && !d.confirm_close) =>
            {
                if let Some(Dialog::Diff(d)) = &mut self.dialog {
                    d.confirm_close = true;
                }
            }
            Message::Quit if self.job.is_some() || !self.queue.is_empty() => {
                // A job's question steps aside (its reply kept); any other dialog gives way.
                if let Some(
                    d @ (Dialog::Conflict { .. } | Dialog::Error { .. } | Dialog::Password { .. }),
                ) = self.dialog.take()
                {
                    self.held = Some(d);
                }
                self.dialog = Some(Dialog::ConfirmExit);
            }
            Message::Quit => return self.handle(Message::Exit),
            Message::Exit => {
                let mine = archive::temp_root().join(std::process::id().to_string());
                let _ = std::fs::remove_dir_all(mine);
                return cosmic::iced::exit();
            }
        }
        Task::none()
    }
}
