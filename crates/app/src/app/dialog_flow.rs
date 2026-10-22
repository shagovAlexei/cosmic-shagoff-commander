//! Dialogs: which one an action opens (`dialog_for`) and what OK does (`submit_dialog`).

use super::*;

impl App {
    /// The dialog an action opens, if any (nothing when there is nothing to act on).
    pub(super) fn dialog_for(&self, side: usize, action: Action) -> Option<Dialog> {
        let panel = &self.panes[side].active().panel;
        let input = |op, sources, input| Dialog::Input {
            op,
            side,
            sources,
            input,
        };
        match action {
            Action::SaveMarksToFile if !panel.has_marks() => None,
            Action::SaveMarksToFile => {
                Some(input(InputOp::SaveMarks, Vec::new(), marks_file(panel)))
            }
            Action::LoadMarksFromFile => {
                Some(input(InputOp::LoadMarks, Vec::new(), marks_file(panel)))
            }
            Action::SaveDetails | Action::SaveDetailsHdr
                if panel.entries().iter().all(|e| e.name == PARENT) =>
            {
                None
            }
            Action::SaveDetails | Action::SaveDetailsHdr => Some(input(
                InputOp::SaveDetails(action == Action::SaveDetailsHdr),
                Vec::new(),
                offered_file(panel, DETAILS_FILE),
            )),
            Action::SelectGroup | Action::UnselectGroup | Action::ShowMask => Some(Dialog::Mask {
                side,
                op: match action {
                    Action::SelectGroup => MaskOp::Select,
                    Action::UnselectGroup => MaskOp::Unselect,
                    _ => MaskOp::Show,
                },
                input: match panel.show() {
                    Show::Mask(m) if action == Action::ShowMask => m.clone(),
                    _ => self.masks.first().cloned().unwrap_or_else(|| "*.*".into()),
                },
            }),
            Action::Copy | Action::Move | Action::Link => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let op = match action {
                    Action::Copy => InputOp::Copy,
                    Action::Link => InputOp::Link,
                    _ => InputOp::Move,
                };
                Some(input(
                    op,
                    sources,
                    dir_input(self.panes[1 - side].active().panel.cwd()),
                ))
            }
            Action::Properties => {
                let paths = panel.targets();
                if let Some((arc, _)) = archive::split_path(panel.cwd()) {
                    return (!paths.is_empty()).then(|| self.archive_props(side, arc, paths));
                }
                (!paths.is_empty()).then(|| self.props_dialog(side, paths))
            }
            // TC: marked files get a rename mask, copied in this dir (`*.bak`, `copy_*`).
            // Not in search results: those come from many dirs, and «this dir» is only the root.
            Action::CopySame
                if panel.has_marks()
                    && panel
                        .targets()
                        .iter()
                        .all(|p| p.parent() == Some(panel.cwd())) =>
            {
                Some(input(InputOp::Copy, panel.targets(), "*.*".to_string()))
            }
            Action::CopySame => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                let path = panel.cwd().join(&e.os_name);
                // In search results the shown name is a path; offer the file's own name.
                let name = path
                    .file_name()
                    .map_or(e.name.clone(), |n| n.to_string_lossy().into_owned());
                let parent = path.parent()?.to_path_buf();
                Some(input(InputOp::Copy, vec![path], {
                    // Relative to the panel's dir: in search results that is not the file's dir.
                    if parent == panel.cwd() {
                        name
                    } else {
                        parent.join(name).to_string_lossy().into_owned()
                    }
                }))
            }
            Action::Mkdir => Some(input(InputOp::Mkdir, Vec::new(), String::new())),
            // TC offers the file under the cursor (an existing one is just opened).
            Action::NewFile => {
                let name = panel
                    .current()
                    .filter(|e| !e.is_dir() && e.name != PARENT)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                Some(input(InputOp::NewFile, Vec::new(), name))
            }
            // The caption as shown; left as the dir's name it keeps following the dir.
            Action::TabRename => {
                let t = self.panes[side].active();
                let shown = t.name.clone();
                let shown = shown.unwrap_or_else(|| format::dir_title(t.panel.cwd()));
                Some(input(InputOp::TabName, Vec::new(), shown))
            }
            Action::Rename => {
                let e = panel.current().filter(|e| e.name != PARENT)?;
                let path = panel.cwd().join(&e.os_name);
                // In search results the shown name is a path; offer the file's own name.
                let name = path
                    .file_name()
                    .map_or(e.name.clone(), |n| n.to_string_lossy().into_owned());
                Some(input(InputOp::Rename, vec![path], name))
            }
            Action::Delete | Action::DeletePermanent => {
                let paths = panel.targets();
                // An archive has no trash: deleting from one is always for good (TC too); in the
                // trash itself, too.
                let in_archive = archive::split_path(panel.cwd()).is_some();
                let for_good = in_archive || paths.iter().any(|p| trashcan::inside(p));
                (!paths.is_empty()).then_some(Dialog::ConfirmDelete {
                    side,
                    permanent: action == Action::DeletePermanent || for_good,
                    paths,
                })
            }
            Action::Hotlist => Some(Dialog::List {
                kind: ListKind::Hotlist,
                side,
                cursor: 0,
                items: self.hotlist_items(),
            }),
            Action::HistoryList => Some(Dialog::List {
                kind: ListKind::History,
                side,
                cursor: 0,
                items: self.panes[side]
                    .active()
                    .history
                    .recent()
                    .into_iter()
                    .map(|p| ListItem {
                        label: format::dir_title(&p),
                        path: p,
                        kind: Item::Dir,
                        icon: String::new(),
                    })
                    .collect(),
            }),
            // A copy: mounts are re-read on every listing and must not shift under the cursor.
            Action::Drives(s) => Some(Dialog::List {
                kind: ListKind::Drives,
                side: s,
                cursor: drives::containing(&self.drives, self.panes[s].active().panel.cwd())
                    .unwrap_or(0),
                items: self
                    .drives
                    .iter()
                    .map(|d| ListItem {
                        label: d.label.clone(),
                        path: d.path.clone(),
                        kind: Item::Dir,
                        icon: String::new(),
                    })
                    .collect(),
            }),
            Action::Connect => Some(Dialog::Connect {
                side,
                url: "sftp://".into(),
                password: String::new(),
                keep: false,
                saved: self.config.connections.clone(),
                found: Vec::new(),
                browsing: None,
                at: mount::NETWORK.to_string(),
                note: None,
            }),
            Action::MultiRename => {
                let wanted: HashSet<PathBuf> = panel.targets().into_iter().collect();
                // A non-UTF-8 name can't round-trip through the text masks; leave it out.
                let files: Vec<(String, SystemTime)> = panel
                    .entries()
                    .iter()
                    .filter(|e| {
                        e.os_name.to_str().is_some()
                            && wanted.contains(&panel.cwd().join(&e.os_name))
                    })
                    .map(|e| (e.name.clone(), e.mtime))
                    .collect();
                if files.is_empty() {
                    return None;
                }
                let dir = panel.cwd().to_path_buf();
                // Hidden files count too, even when not shown (in an archive: its own index);
                // fall back to the listing.
                let taken = match archive::split_path(&dir) {
                    Some((a, inner)) => archive::list(&a, &inner, true).map(|v| {
                        (v.into_iter())
                            .map(|e| e.name)
                            .filter(|n| !files.iter().any(|(f, _)| f == n))
                            .collect()
                    }),
                    None => multirename::other_names(&dir, &files),
                }
                .unwrap_or_else(|_| {
                    panel
                        .entries()
                        .iter()
                        .filter(|e| e.name != PARENT && !files.iter().any(|(n, _)| *n == e.name))
                        .map(|e| e.name.clone())
                        .collect()
                });
                let current = panel
                    .current()
                    .filter(|e| files.iter().any(|(n, _)| *n == e.name))
                    .map(|e| e.name.clone());
                Some(Dialog::MultiRename(Box::new(dialogs::MultiRename {
                    side,
                    dir,
                    files,
                    taken,
                    current,
                    rule: Rule::default(),
                    start: "1".into(),
                    step: "1".into(),
                    digits: "1".into(),
                    help: false,
                })))
            }
            Action::Pack => {
                let sources = panel.targets();
                if sources.is_empty() {
                    return None;
                }
                let format = Format::from_ext(&self.config.pack_format).unwrap_or(Format::Zip);
                let base = panel.cwd().to_path_buf();
                let dirs: Vec<PathBuf> = (panel.entries().iter())
                    .filter(|e| e.is_dir())
                    .map(|e| base.join(&e.os_name))
                    .filter(|p| sources.contains(p))
                    .collect();
                let name = archive::archive_name(&sources, &base, format, &|p| {
                    dirs.iter().any(|d| d == p)
                });
                let path = self.panes[1 - side].active().panel.cwd().join(name);
                Some(Dialog::Pack(Box::new(dialogs::Pack {
                    side,
                    sources,
                    dirs,
                    base,
                    path: path.display().to_string(),
                    format,
                    move_after: false,
                    separate: false,
                    encrypt: false,
                    password: String::new(),
                    again: String::new(),
                })))
            }
            Action::FindFiles => {
                // Inside an archive: search where the archive lies.
                let cwd = panel.cwd();
                let dir = match archive::split_path(cwd) {
                    Some((a, _)) => a.parent().map_or(cwd.to_path_buf(), Path::to_path_buf),
                    None => cwd.to_path_buf(),
                };
                Some(Dialog::Find(Box::new(dialogs::Find {
                    side,
                    mask: self.find.mask.clone(),
                    dir: dir.display().to_string(),
                    find_text: self.find.find_text,
                    help: false,
                    text: self.find.text.clone(),
                    case_sensitive: self.find.case_sensitive,
                    regex: self.find.regex,
                    name_regex: self.find.name_regex,
                    archives: self.find.archives,
                    min_size: self.find.min_size.clone(),
                    max_size: self.find.max_size.clone(),
                    days: self.find.days.clone(),
                    attrs: self.find.attrs.clone(),
                    error: None,
                    root: None,
                    results: Vec::new(),
                    total: 0,
                    current: String::new(),
                    cursor: 0,
                    id: 0, // set by each search start
                    stop: None,
                    in_list: false,
                })))
            }
            Action::CompareFiles => {
                let (left, right) = self.diff_pair(side)?;
                Some(Dialog::Diff(Box::new(dialogs::DiffDlg {
                    left,
                    right,
                    id: 0,
                    result: None,
                    block: 0,
                    scroll: [widget::Id::unique(), widget::Id::unique()],
                    x: 0.0,
                    offset: 0.0,
                    widths: (420.0, 420.0),
                    opts: Default::default(),
                    dirty: (false, false),
                    confirm_close: false,
                    stamps: None,
                    copying: None,
                    undo: Vec::new(),
                    editing: None,
                    edit_id: widget::Id::unique(),
                })))
            }
            Action::SyncDirs => Some(Dialog::Sync(Box::new(dialogs::SyncDlg {
                side,
                left: self.panes[0].active().target(),
                right: self.panes[1].active().target(),
                recursive: self.sync.recursive,
                content: self.sync.content,
                ignore_date: self.sync.ignore_date,
                hidden: panel.show_hidden(),
                hide: self.sync.hide.iter().copied().collect(),
                mirror: false,
                mask: self.sync.mask.clone(),
                confirm: false,
                rows: Vec::new(),
                id: 0,
                running: None,
            }))),
            Action::ChecksumCreate => {
                let sources = panel.targets();
                (!sources.is_empty()).then(|| Dialog::Checksum {
                    side,
                    sources,
                    algo: (Algo::ALL.into_iter())
                        .find(|a| a.ext() == self.config.checksum_algo)
                        .unwrap_or(Algo::Sha256),
                    separate: false,
                })
            }
            Action::Unpack => {
                let archives: Vec<PathBuf> = panel
                    .targets()
                    .into_iter()
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| Format::detect(&n.to_string_lossy()).is_some())
                    })
                    .collect();
                (!archives.is_empty()).then(|| Dialog::Unpack {
                    side,
                    archives,
                    path: dir_input(self.panes[1 - side].active().panel.cwd()),
                    own_dir: false,
                })
            }
            _ => None,
        }
    }

    pub(super) fn submit_dialog(&mut self) -> Task<Message> {
        let Some(d) = self.dialog.take() else {
            return Task::none();
        };
        match d {
            Dialog::Checksum {
                side,
                sources,
                algo,
                separate,
            } => {
                let t = self.panes[side].active();
                let (tab, dir, stop) = (t.id, t.panel.cwd().to_path_buf(), t.counting.clone());
                self.say(StatusKind::Busy, fl!("checksum-making"));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            // Stopped: "" (no error to show).
                            (checksum::create(&dir, &sources, algo, separate, &stop)).map_err(|e| {
                                if e.kind() == std::io::ErrorKind::Interrupted {
                                    String::new()
                                } else {
                                    crate::why::text(&e)
                                }
                            })
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |r| cosmic::Action::App(Message::Summed(tab, r)),
                )
            }
            Dialog::Tree { side, tree, .. } => match tree.current() {
                Some(n) => {
                    self.active = side;
                    self.load(side, n.path.clone(), None)
                }
                None => Task::none(),
            },
            Dialog::Props(p) => {
                // Only the touched bits, as shown now: entries keep the rest of their own modes.
                let (set, clear) = (p.mode & p.touched, !p.mode & p.touched & 0o7777);
                let (uid, gid) = (p.new_user(), p.new_group());
                if p.touched == 0 && uid.is_none() && gid.is_none() {
                    return Task::none();
                }
                let me = shagoff_core::owners::euid();
                // A new owner, a group not the user's, or files not his need root: polkit asks
                // (163), the program itself runs as root to do it.
                let as_root = me != 0
                    && (uid.is_some()
                        || p.foreign
                        || gid.is_some_and(|g| !shagoff_core::owners::my_groups().contains(&g)));
                let (side, paths, recursive) = (p.side, p.paths.clone(), p.recursive);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            use shagoff_core::props;
                            if !as_root {
                                let failed =
                                    props::apply(&paths, uid, gid, (set, clear), recursive);
                                return (failed.into_iter())
                                    .map(|(p, e)| format!("{}: {e}", p.display()))
                                    .collect();
                            }
                            let out = std::env::current_exe().and_then(|exe| {
                                std::process::Command::new("pkexec")
                                    .arg(exe)
                                    .args(props::helper_args(
                                        &paths,
                                        uid,
                                        gid,
                                        (set, clear),
                                        recursive,
                                    ))
                                    .stdin(std::process::Stdio::null())
                                    .output()
                            });
                            match out {
                                Err(e) => vec![format!("pkexec: {e}")],
                                Ok(o) if o.status.success() => Vec::new(),
                                Ok(o) => {
                                    // The helper's failures, or pkexec's refusal.
                                    let err = String::from_utf8_lossy(&o.stderr);
                                    let mut v: Vec<String> =
                                        err.lines().map(str::to_string).collect();
                                    if v.is_empty() {
                                        v.push(format!("pkexec: {}", o.status));
                                    }
                                    v
                                }
                            }
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |failed| cosmic::Action::App(Message::PropsDone(side, failed)),
                )
            }
            Dialog::Mask { side, op, input } => {
                self.masks = cmdline::remember(&self.masks, &input);
                let panel = &mut self.panes[side].active_mut().panel;
                match op {
                    MaskOp::Show => panel.set_show(Show::Mask(input)),
                    _ => panel.mark_by_mask(&Mask::parse(&input), op == MaskOp::Select),
                }
                Task::none()
            }
            Dialog::Input {
                op: InputOp::Mkdir,
                side,
                input,
                ..
            } => self.mkdir(side, input.trim()),
            Dialog::Input {
                op: InputOp::NewFile,
                side,
                input,
                ..
            } => self.new_file(side, input.trim()),
            Dialog::Input {
                op: InputOp::FromTemplate,
                side,
                input,
                sources,
            } => {
                let name = input.trim();
                if !ops::valid_name(name) {
                    if !name.is_empty() {
                        self.say(StatusKind::Error, plan_error(&PlanError::BadName));
                    }
                    return Task::none();
                }
                let cwd = self.panes[side].active().panel.cwd().to_path_buf();
                let Some(template) = sources.first() else {
                    return Task::none();
                };
                if let Err(e) = templates::create(template, &cwd.join(name)) {
                    let err = crate::why::text(&e);
                    self.say(
                        StatusKind::Error,
                        fl!("new-file-failed", path = name, err = err),
                    );
                    return Task::none();
                }
                // `load`: out of search results too, on the new file.
                self.load(side, cwd, Some(name.to_string()))
            }
            Dialog::Input {
                op: op @ (InputOp::SaveMarks | InputOp::LoadMarks),
                side,
                input,
                ..
            } => {
                let file = writable_dir(&self.panes[side].active().panel)
                    .join(session::expand_home(Path::new(input.trim()), &self.home));
                let panel = &mut self.panes[side].active_mut().panel;
                let done = match op {
                    InputOp::SaveMarks => {
                        let names = panel.marked_names();
                        let n = names.len();
                        write_new(&file, &(names.join("\n") + "\n")).map(|_| n)
                    }
                    _ => std::fs::read_to_string(&file).map(|t| panel.mark_listed(&t)),
                };
                match (done, op) {
                    (Ok(n), InputOp::SaveMarks) => {
                        self.say(StatusKind::Info, fl!("marks-saved", count = n))
                    }
                    (Ok(n), _) => self.say(StatusKind::Info, fl!("marks-loaded", count = n)),
                    (Err(e), _) => self.say(StatusKind::Error, crate::why::text(&e)),
                }
                Task::none()
            }
            Dialog::Input {
                op: InputOp::SaveDetails(head),
                side,
                input,
                ..
            } => {
                let panel = &self.panes[side].active().panel;
                let file = writable_dir(&self.panes[side].active().panel)
                    .join(session::expand_home(Path::new(input.trim()), &self.home));
                let rows: Vec<&Entry> = (panel.entries().iter())
                    .filter(|e| e.name != PARENT)
                    .collect();
                let n = rows.len();
                let text = self.details_text(panel, &rows, head);
                match write_new(&file, &text) {
                    Ok(()) => self.say(StatusKind::Info, fl!("details-saved", count = n)),
                    Err(e) => self.say(StatusKind::Error, crate::why::text(&e)),
                }
                Task::none()
            }
            Dialog::Input {
                op: InputOp::OpenWith,
                side,
                sources,
                input,
            } => {
                if input.trim().is_empty() {
                    return Task::none();
                }
                self.other_cmds = cmdline::remember(&self.other_cmds, &input);
                let dir = self.panes[side].active().panel.cwd().to_path_buf();
                let line = openwith::other_line(&input, &sources);
                // Watched like a toolbar command: a typo ("gimpp") is reported, not silent.
                match run_watched(&cmdline::argv(&line, &dir, None), &dir) {
                    Ok(task) => task,
                    Err(e) => {
                        self.say(StatusKind::Error, fl!("cmd-failed", error = e.to_string()));
                        Task::none()
                    }
                }
            }
            Dialog::Input {
                op: InputOp::TabName,
                side,
                input,
                ..
            } => {
                let name = input.trim();
                let t = self.panes[side].active_mut();
                let own = !name.is_empty() && name != format::dir_title(t.panel.cwd());
                t.name = own.then(|| name.to_string());
                Task::none()
            }
            Dialog::Input {
                op,
                side,
                sources,
                input,
            } => self.start_transfer(op, side, sources, input.trim()),
            Dialog::ConfirmDelete {
                side,
                permanent,
                paths,
            } => self.start_delete(side, permanent, paths),
            Dialog::MultiRename(m) => {
                let rows = m.rows(&self.tz);
                if rows.iter().any(|r| r.problem.is_some()) {
                    self.dialog = Some(Dialog::MultiRename(m)); // Enter does nothing until fixed
                    return Task::none();
                }
                // In an archive: paths inside it, for the repack.
                let inner = archive::split_path(&m.dir);
                let base = inner.as_ref().map_or(m.dir.as_path(), |(_, i)| i.as_path());
                let pairs = multirename::plan(base, &rows);
                if pairs.is_empty() {
                    return Task::none();
                }
                let focus = m
                    .current
                    .as_ref()
                    .and_then(|c| rows.iter().find(|r| &r.old == c))
                    .map(|r| r.new.clone());
                if let Some((archive, _)) = inner {
                    let job = Job::Repack {
                        archive,
                        change: Change::Rename(pairs),
                    };
                    return self.start_job(m.side, OpKind::Repack, job, focus);
                }
                let job = Job::Transfer {
                    method: Method::Rename,
                    pairs,
                };
                self.start_job(m.side, OpKind::Move, job, focus)
            }
            Dialog::Pack(p) => {
                let Ok(password) = p.password() else {
                    self.dialog = Some(Dialog::Pack(p)); // Enter does nothing until they match
                    return Task::none();
                };
                let cwd = self.panes[p.side].active().target();
                let path = cwd.join(p.path.trim());
                if into_archive(&path) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
                    return Task::none();
                }
                let is_dir = |s: &Path| p.dirs.iter().any(|d| d == s);
                let groups =
                    archive::groups(&p.sources, &p.base, &path, p.format, p.separate, &is_dir);
                // Cursor on the new archive when it lands in this panel.
                let focus = match groups.as_slice() {
                    [(_, a)] if a.parent() == Some(cwd.as_path()) => {
                        a.file_name().map(|n| n.to_string_lossy().into_owned())
                    }
                    _ => None,
                };
                let job = Job::Pack {
                    format: p.format,
                    base: p.base,
                    groups,
                    move_after: p.move_after,
                    password,
                };
                self.start_job(p.side, OpKind::Pack, job, focus)
            }
            Dialog::Connect {
                side,
                url,
                password,
                keep,
                saved,
                found,
                browsing,
                at,
                note,
            } => {
                let address = url.trim().to_string();
                // Stays open while connecting: the error lands in it, Cancel stops it.
                let busy = self.connecting.is_some();
                self.dialog = Some(Dialog::Connect {
                    side,
                    url,
                    password: password.clone(),
                    keep,
                    saved,
                    found,
                    browsing,
                    at,
                    note: if busy || address.is_empty() {
                        note
                    } else {
                        Some((false, fl!("connecting")))
                    },
                });
                if busy || address.is_empty() {
                    return Task::none();
                }
                let url = address;
                let cancel = Arc::new(AtomicBool::new(false));
                self.connecting = Some((url.clone(), cancel.clone()));
                self.say(StatusKind::Busy, fl!("connecting"));
                let address = url.clone();
                blocking(
                    move || mount::connect(&url, &password, &cancel),
                    move |r| Message::Connected(side, address, r),
                )
            }
            Dialog::Unpack {
                side,
                archives,
                path,
                own_dir,
            } => {
                let dest = self.panes[side].active().target().join(path.trim());
                if into_archive(&dest) {
                    self.say(StatusKind::Error, fl!("archive-read-only"));
                    return Task::none();
                }
                let job = Job::Unpack {
                    archives,
                    dest,
                    own_dir,
                };
                self.start_job(side, OpKind::Unpack, job, None)
            }
            d @ Dialog::Sync(_) => {
                self.dialog = Some(d);
                self.start_sync()
            }
            // Read-only view: Enter does nothing.
            d @ Dialog::Diff(_) => {
                self.dialog = Some(d);
                Task::none()
            }
            d @ Dialog::Find(_) => {
                self.dialog = Some(d);
                self.start_find()
            }
            Dialog::Hotlist(h) => {
                self.save_hotlist(h.list);
                Task::none()
            }
            Dialog::Colors(c) => {
                match &self.config_handler {
                    Some(h) => {
                        let colors = self.config.set_colors(h, c.list);
                        let panel = self.config.set_panel(h, c.panel);
                        if let Err(e) = colors.and(panel) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => (self.config.colors, self.config.panel) = (c.list, c.panel),
                }
                self.scheme = shagoff_core::colors::Scheme::new(&self.config.colors)
                    .with_panel(&self.config.panel);
                Task::none()
            }
            Dialog::Toolbar(mut t) if t.menu => {
                t.list.retain(|b| !b.cmd.trim().is_empty()); // added, never filled in
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_start_menu(h, t.list) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.start_menu = t.list,
                }
                Task::none()
            }
            Dialog::Toolbar(t) => {
                self.save_toolbar(t.list);
                Task::none()
            }
            Dialog::Keys(k) => {
                let list = k.result();
                match &self.config_handler {
                    Some(h) => {
                        if let Err(e) = self.config.set_hotkeys(h, list) {
                            self.config_not_saved(&e);
                        }
                    }
                    None => self.config.hotkeys = list,
                }
                keymap::set_own(crate::hotkeys::resolve(&self.config.hotkeys));
                Task::none()
            }
            d @ Dialog::List { .. } => {
                let i = match &d {
                    Dialog::List { cursor, .. } => *cursor,
                    _ => 0,
                };
                self.dialog = Some(d);
                self.pick(i)
            }
            Dialog::UpdateArchive {
                file,
                archive,
                entry,
            } => {
                self.updating = Some(file.clone());
                let job = Job::Repack {
                    archive,
                    change: Change::Replace { entry, file },
                };
                self.start_job(self.active, OpKind::Repack, job, None)
            }
            // Exits once the worker has stopped and cleaned its part file (finish_job).
            Dialog::ConfirmExit => {
                self.cancel_job();
                self.quitting = true;
                if self.job.is_none() {
                    return self.handle(Message::Exit);
                }
                Task::none()
            }
            Dialog::Password {
                archive,
                password,
                reply,
                open,
                ..
            } => {
                archive::set_password(&archive, Some(password));
                if let Some(r) = reply {
                    let _ = r.send(ErrorChoice::Retry);
                }
                match open {
                    Some((side, dir)) => self.load(side, dir, None),
                    None => Task::none(),
                }
            }
            // Answered with their own buttons, not Enter/OK.
            d @ (Dialog::Conflict { .. } | Dialog::Error { .. } | Dialog::DropMenu { .. }) => {
                self.dialog = Some(d);
                Task::none()
            }
        }
    }

    /// Shift+F4: create `name` in the panel's dir if it is not there, open it in the editor,
    /// put the cursor on it.
    pub(super) fn new_file(&mut self, side: usize, name: &str) -> Task<Message> {
        if !ops::valid_name(name) {
            if !name.is_empty() {
                self.say(StatusKind::Error, plan_error(&PlanError::BadName));
            }
            return Task::none();
        }
        let cwd = self.panes[side].active().panel.cwd().to_path_buf();
        let file = cwd.join(name);
        if let Err(e) = ops::new_file(&file) {
            let err = crate::why::text(&e);
            self.say(
                StatusKind::Error,
                fl!("new-file-failed", path = name, err = err),
            );
            return Task::none();
        }
        let argv = launch::command(&self.config.editor, &["cosmic-edit"], &file);
        if let Err(err) = spawn_detached(&argv) {
            self.say(
                StatusKind::Error,
                fl!("open-failed", err = crate::why::text(&err)),
            );
        }
        self.reload(side, cwd, Some(name.to_string()))
    }

    /// F7: create (possibly nested) dirs, then put the cursor on the first created component.
    pub(super) fn mkdir(&mut self, side: usize, name: &str) -> Task<Message> {
        if name.is_empty() {
            return Task::none();
        }
        let t = self.panes[side].active_mut();
        let cwd = t.panel.cwd().to_path_buf();
        let first = || match Path::new(name).components().next() {
            Some(Component::Normal(first)) => Some(first.to_string_lossy().into_owned()),
            _ => None,
        };
        if let Some((archive, inner)) = archive::split_path(&cwd) {
            if Path::new(name).is_absolute() {
                self.say(StatusKind::Error, plan_error(&PlanError::BadName));
                return Task::none();
            }
            let job = Job::Repack {
                archive,
                change: Change::Mkdir(inner.join(name)),
            };
            return self.start_job(side, OpKind::Repack, job, first());
        }
        // In the background: on a dead network mount `mkdir` never returns.
        let (dir, name, first) = (cwd.join(name), name.to_string(), first());
        Task::perform(
            async move {
                let made = tokio::task::spawn_blocking(move || std::fs::create_dir_all(dir))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| crate::why::text(&e)));
                Message::Made {
                    side,
                    cwd,
                    name,
                    first,
                    made,
                }
            },
            cosmic::Action::App,
        )
    }

    /// Ctrl+Shift+F5: links placed by F5's rules (`link_all`, in the background); no job, no
    /// dialog. Both panes reread.
    pub(super) fn make_links(
        &mut self,
        side: usize,
        sources: &[PathBuf],
        target: &Path,
    ) -> Task<Message> {
        if [sources.first().map(PathBuf::as_path), Some(target)]
            .into_iter()
            .flatten()
            .any(inside_archive)
        {
            self.say(StatusKind::Error, fl!("archive-read-only"));
            return Task::none();
        }
        // In the background: on a dead network mount a `stat` or `symlink` never returns.
        let (sources, target) = (sources.to_vec(), target.to_path_buf());
        let tab = self.panes[side].active().id;
        Task::perform(
            async move {
                let made = tokio::task::spawn_blocking(move || link_all(&sources, &target))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                Message::Linked(tab, made)
            },
            cosmic::Action::App,
        )
    }
}
