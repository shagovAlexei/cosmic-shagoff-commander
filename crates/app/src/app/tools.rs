//! File operations and tools started from the panels: copy / move, compare, sync, properties, find, archives, branch view.

use super::*;

impl App {
    /// F5 / F6 / rename: resolve the target, then run the transfer on a worker thread.
    pub(super) fn start_transfer(
        &mut self,
        op: InputOp,
        side: usize,
        sources: Vec<PathBuf>,
        input: &str,
    ) -> Task<Message> {
        // Search results in the panel: files from archives go out one archive dir at a time (each
        // its own job, queued), the ones on disk together.
        if op != InputOp::Rename {
            let groups = archive::split_by(&sources, |p| {
                p.parent()
                    .filter(|d| archive::within(d))
                    .map(Path::to_path_buf)
            });
            if groups.len() > 1 {
                let tasks: Vec<_> = (groups.into_iter())
                    .map(|g| self.start_transfer(op, side, g, input))
                    .collect();
                return Task::batch(tasks);
            }
        }
        // While a navigation is in flight the rows still show the dir being left; aim at where the tab is going.
        let cwd = self.panes[side].active().target();
        // From the rows' own dir: right after Enter on an archive they still show the dir left.
        let from = sources
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let from_archive = from.as_deref().and_then(archive::split_path);
        let target = cwd.join(session::expand_home(Path::new(input), &self.home));
        if op == InputOp::Link {
            return self.make_links(side, &sources, &target);
        }
        // The input's trailing `/` ("into this dir", TC) kept for `Change::Add`.
        let to_archive = (op != InputOp::Rename)
            .then(|| archive::split_path(&target))
            .flatten()
            .map(|(a, mut inner)| {
                if input.ends_with('/') && !inner.as_os_str().is_empty() {
                    inner.as_mut_os_string().push("/");
                }
                (a, inner)
            });
        match (from_archive, to_archive) {
            (Some((from, inner)), Some((to, to_inner))) => {
                let names: Vec<PathBuf> = (sources.iter())
                    .filter_map(|p| p.file_name().map(PathBuf::from))
                    .collect();
                // Inside one archive, as on disk: not onto itself, not a dir into itself (by the
                // real file: the same archive may be reached through a link).
                let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                if real(&from) == real(&to) {
                    // Into its own dir, or one file onto itself by name (`a.zip/x` → `a.zip/x`).
                    let onto_itself = matches!(&names[..], [n] if inner.join(n) == to_inner);
                    let e = if inner == to_inner || onto_itself {
                        sources.first().map(|s| PlanError::SameFile(s.clone()))
                    } else {
                        (names.iter().map(|n| inner.join(n)))
                            .find(|n| to_inner.starts_with(n))
                            .map(|n| PlanError::IntoItself(from.join(n)))
                    };
                    if let Some(e) = e {
                        self.say(StatusKind::Error, plan_error(&e));
                        return Task::none();
                    }
                }
                let job = Job::Cross {
                    from,
                    inner,
                    names,
                    to,
                    to_inner,
                    move_after: op == InputOp::Move,
                };
                return self.start_job(side, OpKind::Repack, job, None);
            }
            (Some((archive, inner)), None) if op == InputOp::Rename => {
                let Some(name) = sources.first().and_then(|p| p.file_name()) else {
                    return Task::none();
                };
                if input.is_empty() || Path::new(name) == Path::new(input) {
                    return Task::none();
                }
                // As outside archives (`rename_pairs`): a new name, not a path.
                if input.contains('/') || input == "." || input == ".." {
                    self.say(StatusKind::Error, plan_error(&PlanError::BadName));
                    return Task::none();
                }
                let job = Job::Repack {
                    archive,
                    change: Change::Rename(vec![(inner.join(name), inner.join(input))]),
                };
                return self.start_job(side, OpKind::Repack, job, Some(input.to_string()));
            }
            (Some(_), None) => {
                let from = from.unwrap_or_default();
                return self.start_extract(side, &from, &sources, target, op == InputOp::Move);
            }
            (None, Some((archive, inner))) => {
                // The archive itself (or a dir holding it) among the sources: F6 would delete it.
                let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                let arc = real(&archive);
                if let Some(s) = sources.iter().find(|s| arc.starts_with(real(s))) {
                    let e = PlanError::IntoItself(s.clone());
                    self.say(StatusKind::Error, plan_error(&e));
                    return Task::none();
                }
                let job = Job::Repack {
                    archive,
                    change: Change::Add {
                        sources,
                        inner,
                        move_sources: op == InputOp::Move,
                    },
                };
                return self.start_job(side, OpKind::Repack, job, None);
            }
            (None, None) => {}
        }
        let (method, kind) = match op {
            InputOp::Copy => (Method::Copy, OpKind::Copy),
            _ => (Method::Move, OpKind::Move),
        };
        let (planned, focus) = if op == InputOp::Rename {
            // Not `plan`: an existing dir name must be refused, not moved into or merged with.
            let Some(src) = sources.first() else {
                return Task::none();
            };
            (ops::rename_pairs(src, input), Some(input.to_string()))
        } else {
            (ops::plan(&sources, &target), None)
        };
        let pairs = match planned {
            Ok(pairs) if pairs.is_empty() => return Task::none(), // rename to the same name
            Ok(pairs) => pairs,
            Err(e) => {
                self.say(StatusKind::Error, plan_error(&e));
                return Task::none();
            }
        };
        let replan = (op != InputOp::Rename).then_some((sources, target));
        self.start_queued(side, kind, Job::Transfer { method, pairs }, focus, replan)
    }

    /// Ctrl+Shift+D: two marked files of this panel, else the files under both cursors.
    pub(super) fn diff_pair(&self, side: usize) -> Option<(PathBuf, PathBuf)> {
        let file = |s: usize| {
            let p = &self.panes[s].active().panel;
            p.current()
                .filter(|e| !e.is_dir() && e.name != PARENT)
                .map(|e| p.cwd().join(&e.os_name))
        };
        // Paths through an archive are not real files: extract first (F5).
        if (0..2).any(|s| archive::split_path(&self.panes[s].active().target()).is_some()) {
            return None;
        }
        let panel = &self.panes[side].active().panel;
        let marked: Vec<&Entry> = panel
            .entries()
            .iter()
            .filter(|e| panel.is_marked(e))
            .collect();
        match marked.as_slice() {
            [] => Some((file(0)?, file(1)?)),
            [a, b] if !a.is_dir() && !b.is_dir() => {
                Some((panel.cwd().join(&a.os_name), panel.cwd().join(&b.os_name)))
            }
            // Something is marked, but not two files: don't silently compare other ones.
            _ => None,
        }
    }

    pub(super) fn start_diff(&mut self) -> Task<Message> {
        self.start_diff_with(None, None, None)
    }

    /// Compare the dialog's files: read them (`src` None), or the texts in hand with new options
    /// or after copying block `copy.0` (`copy.1`: left → right), or the texts `undo` (a copy undone).
    pub(super) fn start_diff_with(
        &mut self,
        shown: Option<Arc<Result<shagoff_core::diff::Outcome, String>>>,
        copy: Option<(usize, bool)>,
        undo: Option<Arc<(String, String)>>,
    ) -> Task<Message> {
        use shagoff_core::diff;
        let id = self.next_id();
        let Some(Dialog::Diff(d)) = &mut self.dialog else {
            return Task::none();
        };
        d.id = id;
        // Nothing to save or copy from until this result is in (it may hold a copied block).
        d.result = None;
        let (a, b, o) = (d.left.clone(), d.right.clone(), d.opts);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if let Some(src) = undo {
                        return Ok(diff::Outcome::Text(diff::rows_with(
                            &src.0,
                            &src.1,
                            diff::ROWS_LIMIT,
                            o,
                        )));
                    }
                    let Some(Ok(diff::Outcome::Text(t))) = shown.as_deref() else {
                        return diff::compare_with(&a, &b, o).map_err(|e| crate::why::text(&e));
                    };
                    let (mut l, mut r) = (t.src.0.clone(), t.src.1.clone());
                    if let Some((block, to_right)) = copy
                        && let Some(new) = diff::copy_block(t, block, to_right)
                    {
                        if to_right {
                            r = new;
                        } else {
                            l = new;
                        }
                    }
                    Ok(diff::Outcome::Text(diff::rows_with(
                        &l,
                        &r,
                        diff::ROWS_LIMIT,
                        o,
                    )))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |out| cosmic::Action::App(Message::DiffReady(id, Arc::new(out))),
        )
    }

    /// Next / previous block of differences, scrolled into view.
    pub(super) fn diff_step(&mut self, delta: isize) -> Task<Message> {
        let Some(Dialog::Diff(d)) = &mut self.dialog else {
            return Task::none();
        };
        let (Some(t), (top, bottom)) = (d.text(), d.view_rows()) else {
            return Task::none();
        };
        if d.blocks().is_empty() {
            return Task::none();
        }
        d.block = shagoff_core::diff::step(&t.rows, d.blocks(), d.block, delta > 0, top, bottom);
        self.diff_scroll()
    }

    pub(super) fn diff_scroll(&self) -> Task<Message> {
        let Some(Dialog::Diff(d)) = &self.dialog else {
            return Task::none();
        };
        let Some(&row) = d.blocks().get(d.block) else {
            return Task::none();
        };
        // A couple of rows of context above the block; both sides, back to the line starts.
        let y = row.saturating_sub(2) as f32 * dialogs::DIFF_ROW_H;
        let to = AbsoluteOffset {
            x: Some(0.0),
            y: Some(y),
        };
        Task::batch(
            d.scroll
                .iter()
                .map(|id| scrollable::scroll_to(id.clone(), to)),
        )
    }

    /// (Re)compare the sync dialog's dirs in the background; an older compare is stopped.
    pub(super) fn start_compare(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Sync(s)) = &mut self.dialog else {
            return Task::none();
        };
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = s.running.replace(stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        s.id = id;
        let (l, r, o) = (s.left.clone(), s.right.clone(), s.options());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || shagoff_core::sync::compare(&l, &r, &o, &stop))
                    .await
                    .unwrap_or_default()
            },
            move |rows| cosmic::Action::App(Message::SyncCompared(id, rows)),
        )
    }

    /// Copy by the arrows; the dialog closes, both panes reload after the job.
    pub(super) fn start_sync(&mut self) -> Task<Message> {
        let Some(Dialog::Sync(s)) = &mut self.dialog else {
            return Task::none();
        };
        if s.running.is_some() {
            return Task::none();
        }
        let p = shagoff_core::sync::plan(&s.left, &s.right, &s.rows);
        if p.to_right.is_empty() && p.to_left.is_empty() && p.delete.is_empty() {
            return Task::none();
        }
        // Deleting: the button turns into "delete and synchronize" first.
        if !p.delete.is_empty() && !s.confirm {
            s.confirm = true;
            return Task::none();
        }
        let side = s.side;
        // One side inside an archive: what comes out is extracted, what goes in added, each a
        // job of its own (queued).
        if let Some(ap) = shagoff_core::sync::archive_plan(&s.left, &s.right, &s.rows) {
            self.dialog = None;
            let mut tasks = Vec::new();
            if let Some((archive, inner, names, dest)) = ap.extract {
                let job = Job::Extract {
                    archive,
                    inner,
                    names,
                    dest,
                };
                tasks.push(self.start_job(side, OpKind::Extract, job, None));
            }
            for (archive, inner, sources) in ap.add {
                let change = Change::Add {
                    sources,
                    inner,
                    move_sources: false,
                };
                let job = Job::Repack { archive, change };
                tasks.push(self.start_job(side, OpKind::Repack, job, None));
            }
            if let Some((archive, entries)) = ap.delete_in {
                let job = Job::Repack {
                    archive,
                    change: Change::Delete(entries),
                };
                tasks.push(self.start_job(side, OpKind::Repack, job, None));
            }
            if !ap.delete_disk.is_empty() {
                let job = Job::Delete {
                    paths: ap.delete_disk,
                    permanent: false,
                };
                tasks.push(self.start_job(side, OpKind::Delete, job, None));
            }
            return Task::batch(tasks);
        }
        self.dialog = None;
        let job = Job::Sync {
            to_right: p.to_right,
            to_left: p.to_left,
            delete: p.delete,
            permanent: false,
        };
        self.start_job(side, OpKind::Sync, job, None)
    }

    /// Alt+Enter: facts about the selection and its bits (of the first entry that has any).
    pub(super) fn props_dialog(&self, side: usize, paths: Vec<PathBuf>) -> Dialog {
        use std::os::unix::fs::MetadataExt;
        let mut facts = Vec::new();
        let metas: Vec<_> = paths
            .iter()
            .map(|p| std::fs::symlink_metadata(p).ok())
            .collect();
        if let [path] = paths.as_slice() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            facts.push((fl!("props-name"), name.into_owned()));
            if let Some(m) = &metas[0] {
                let kind = if m.is_dir() {
                    fl!("props-folder")
                } else if m.file_type().is_symlink() {
                    let to = std::fs::read_link(path).unwrap_or_default();
                    format!("{} {}", fl!("props-link"), to.display())
                } else {
                    let ext = path.extension().unwrap_or_default().to_string_lossy();
                    format::mime_type(&ext).unwrap_or_else(|| "—".into())
                };
                facts.push((fl!("props-type"), kind));
                let mtime = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                facts.push((fl!("props-modified"), format::date(mtime, &self.tz)));
            }
        } else {
            facts.push((fl!("props-selected"), fl!("props-items", n = paths.len())));
        }
        let modes: Vec<u32> = paths
            .iter()
            .filter_map(|p| shagoff_core::props::mode(p))
            .collect();
        let all = modes.iter().fold(0o7777, |a, m| a & m);
        let any = modes.iter().fold(0, |a, m| a | m);
        let has_dir = metas.iter().flatten().any(|m| m.is_dir());
        // The drop-downs: every group / user and the files' own (a number when not in /etc).
        let common = |ids: &[u32], all: &[(u32, String)]| {
            (ids.first())
                .filter(|g| ids.iter().all(|x| x == *g))
                .and_then(|g| all.iter().position(|(id, _)| id == g))
        };
        let file_gids: Vec<u32> = metas.iter().flatten().map(|m| m.gid()).collect();
        let mut gids = self.owners.all_gids();
        gids.extend(&file_gids);
        let groups = self.owners.groups(&gids);
        let group = common(&file_gids, &groups);
        let file_uids: Vec<u32> = metas.iter().flatten().map(|m| m.uid()).collect();
        let mut uids = self.owners.all_uids();
        uids.extend(&file_uids);
        let users = self.owners.users(&uids);
        let user = common(&file_uids, &users);
        Dialog::Props(Box::new(dialogs::Props {
            side,
            paths,
            facts,
            mode: if modes.is_empty() { 0 } else { all },
            mixed: any & !all,
            own_mixed: any & !all,
            touched: 0,
            has_dir,
            recursive: false,
            gids: groups.iter().map(|(g, _)| *g).collect(),
            group_names: groups.into_iter().map(|(_, n)| n).collect(),
            group,
            group0: group,
            uids: users.iter().map(|(u, _)| *u).collect(),
            user_names: users.into_iter().map(|(_, n)| n).collect(),
            user,
            user0: user,
            foreign: {
                let me = shagoff_core::owners::euid();
                file_uids.iter().any(|u| *u != me)
            },
            usage: None,
            id: 0, // set by `start_usage`
            stop: Arc::new(AtomicBool::new(false)),
            archive: None,
        }))
    }

    /// Alt+Enter inside an archive: what its index says, nothing to change (the bits are shown
    /// greyed, no group, no Apply).
    pub(super) fn archive_props(&self, side: usize, arc: PathBuf, paths: Vec<PathBuf>) -> Dialog {
        let panel = &self.panes[side].active().panel;
        let entries: Vec<&Entry> = (panel.entries().iter())
            .filter(|e| paths.contains(&panel.cwd().join(&e.os_name)))
            .collect();
        let mut facts = Vec::new();
        if let [e] = entries.as_slice() {
            facts.push((fl!("props-name"), e.name.clone()));
            let kind = if e.is_dir() {
                fl!("props-folder")
            } else if e.is_link {
                fl!("props-link-archive")
            } else {
                format::mime_type(&e.ext).unwrap_or_else(|| "—".into())
            };
            facts.push((fl!("props-type"), kind));
            facts.push((fl!("props-modified"), format::date(e.mtime, &self.tz)));
        } else {
            facts.push((fl!("props-selected"), fl!("props-items", n = paths.len())));
        }
        facts.push((fl!("props-archive"), arc.display().to_string()));
        let all = entries.iter().fold(0o7777, |a, e| a & e.mode);
        let any = entries.iter().fold(0, |a, e| a | e.mode);
        Dialog::Props(Box::new(dialogs::Props {
            side,
            paths,
            facts,
            mode: if entries.is_empty() { 0 } else { all },
            mixed: any & !all,
            own_mixed: any & !all,
            touched: 0,
            has_dir: false,
            recursive: false,
            gids: Vec::new(),
            group_names: Vec::new(),
            group: None,
            group0: None,
            uids: Vec::new(),
            user_names: Vec::new(),
            user: None,
            user0: None,
            foreign: false,
            usage: None,
            id: 0,
            stop: Arc::new(AtomicBool::new(false)),
            archive: Some(arc),
        }))
    }

    /// Count the open properties dialog's selection in the background.
    pub(super) fn start_usage(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Props(p)) = &mut self.dialog else {
            return Task::none();
        };
        p.id = id;
        let (paths, stop, arc) = (p.paths.clone(), p.stop.clone(), p.archive.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match arc {
                    Some(a) => {
                        let inner: Vec<PathBuf> = (paths.iter())
                            .filter_map(|p| Some(p.strip_prefix(&a).ok()?.to_path_buf()))
                            .collect();
                        archive::usage_of(&a, &inner).ok()
                    }
                    None => shagoff_core::props::usage(&paths, &stop),
                })
                .await
                .unwrap_or_default()
            },
            move |u| cosmic::Action::App(Message::PropsUsage(id, u)),
        )
    }

    /// (Re)start the search of the open find dialog; the previous one is stopped.
    pub(super) fn start_find(&mut self) -> Task<Message> {
        let id = self.next_id();
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let side = f.side;
        self.find = config::FindPrefs {
            mask: f.mask.clone(),
            find_text: f.find_text,
            text: f.text.clone(),
            case_sensitive: f.case_sensitive,
            regex: f.regex,
            name_regex: f.name_regex,
            archives: f.archives,
            min_size: f.min_size.clone(),
            max_size: f.max_size.clone(),
            days: f.days.clone(),
            attrs: f.attrs.clone(),
        };
        let root = self.panes[side].active().target().join(f.dir.trim());
        let root_searched = root.clone();
        // «Искать текст» off: names only, whatever the field holds.
        let text = if f.find_text {
            f.text.trim().to_string()
        } else {
            String::new()
        };
        let q = match find_query(f, text) {
            Ok(q) => shagoff_core::search::Query {
                hidden: self.panes[side].active().panel.show_hidden(),
                ..q
            },
            Err(e) => {
                // The last search's results would read as this one's under the error.
                if let Some(Dialog::Find(f)) = &mut self.dialog {
                    if let Some(old) = f.stop.take() {
                        old.store(true, Ordering::Relaxed);
                    }
                    // A new id: what the stopped search still sends is not this one's.
                    (f.id, f.cursor, f.total) = (id, 0, 0);
                    f.results.clear();
                    f.current.clear();
                    f.root = None;
                    f.error = Some(e);
                }
                return Task::none();
            }
        };
        let (stop, events) = crate::find::spawn(id, root, q);
        if let Some(Dialog::Find(f)) = &mut self.dialog {
            if let Some(old) = f.stop.replace(stop) {
                old.store(true, Ordering::Relaxed);
            }
            (f.id, f.cursor, f.total, f.in_list) = (id, 0, 0, false);
            f.error = None;
            f.root = Some(root_searched);
            f.results.clear();
        }
        Task::run(events, |e| cosmic::Action::App(Message::Find(e)))
    }

    pub(super) fn on_find_event(&mut self, e: crate::find::FindEvent) {
        use crate::find::FindEvent;
        let Some(Dialog::Find(f)) = &mut self.dialog else {
            return;
        };
        match e {
            FindEvent::Found(id, paths) if id == f.id => {
                f.total += paths.len();
                f.results.extend(paths);
            }
            FindEvent::Dir(id, d) if id == f.id => f.current = d.display().to_string(),
            FindEvent::Done(id) if id == f.id => {
                f.stop = None;
                f.current.clear();
            }
            _ => {} // a previous search
        }
    }

    /// "To panel": the active tab of the dialog's panel lists the results until it is left.
    pub(super) fn find_feed(&mut self) -> Task<Message> {
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let side = f.side;
        // The dir that was searched, not the field as edited since.
        let Some(root) = f.root.clone() else {
            return Task::none();
        };
        let paths = Arc::new(f.results.clone());
        self.dialog = None; // stops the search
        self.active = side;
        self.leave_locked(side);
        let t = self.panes[side].active_mut();
        t.results = Some((root.clone(), paths));
        t.branch = false;
        t.stop_branch(); // a branch read landing later would replace these
        self.reload(side, root, None)
    }

    /// Result i: its dir in the dialog's panel, cursor on it (also for a found dir, as in TC).
    pub(super) fn find_pick(&mut self, i: usize) -> Task<Message> {
        let Some(Dialog::Find(f)) = &self.dialog else {
            return Task::none();
        };
        let (side, Some(path)) = (f.side, f.results.get(i).cloned()) else {
            return Task::none();
        };
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Task::none();
        };
        let (dir, name) = (dir.to_path_buf(), name.to_string_lossy().into_owned());
        self.dialog = None; // stops the search (Drop)
        self.active = side;
        self.load(side, dir, Some(name))
    }

    /// Copy `paths` (inside the archive dir `cwd`) out into the real dir `dest`.
    pub(super) fn start_extract(
        &mut self,
        side: usize,
        cwd: &Path,
        paths: &[PathBuf],
        dest: PathBuf,
        // F6: delete from the archive what came out.
        move_after: bool,
    ) -> Task<Message> {
        let Some((archive, inner)) = archive::split_path(cwd) else {
            return Task::none();
        };
        if paths.is_empty() || into_archive(&dest) {
            return Task::none();
        }
        let names = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(PathBuf::from)
            .collect();
        if move_after {
            let job = Job::ExtractMove {
                archive,
                inner,
                names,
                dest,
            };
            return self.start_job(side, OpKind::Move, job, None);
        }
        let job = Job::Extract {
            archive,
            inner,
            names,
            dest,
        };
        self.start_job(side, OpKind::Extract, job, None)
    }

    /// Enter / F3 on a file inside an archive: extract it to a fresh temp dir, then open it.
    pub(super) fn open_from_archive(
        &mut self,
        side: usize,
        archive: PathBuf,
        inner: PathBuf,
        name: OsString,
        how: How,
    ) -> Task<Message> {
        // Opened right away or not at all: queued, the file would pop up out of nowhere later.
        if self.job.is_some() {
            self.say(StatusKind::Error, fl!("job-running"));
            return Task::none();
        }
        let dir = match archive::fresh_temp_dir(&archive::temp_root()) {
            Ok(d) => d,
            Err(e) => {
                self.say(
                    StatusKind::Error,
                    fl!("open-failed", err = crate::why::text(&e)),
                );
                return Task::none();
            }
        };
        let file = dir.join(&name);
        let open = match how {
            How::Run(prog, default) => Open::Run(launch::command(&prog, default, &file)),
            How::Edit(prog) => Open::Edit {
                argv: launch::command(&prog, &["cosmic-edit"], &file),
                archive: archive.clone(),
                entry: inner.join(&name),
            },
            How::Lister => Open::Lister(name.to_string_lossy().into_owned()),
            How::Browse => Open::Browse(archive.join(&inner)),
        };
        let job = Job::Extract {
            archive,
            inner,
            names: vec![PathBuf::from(name)],
            dest: dir,
        };
        let task = self.start_job(side, OpKind::Extract, job, None);
        if let Some(j) = &mut self.job {
            j.open = Some((open, file));
        }
        task
    }

    /// Ctrl+B / Ctrl+Shift+B (TC "branch"): every file below the dir (or the marked dirs) in one
    /// list, read in the background; again — the dir as usual.
    pub(super) fn branch(&mut self, side: usize, action: Action) -> Task<Message> {
        let t = self.panes[side].active();
        if t.branch
            && let Some((root, _)) = &t.results
        {
            let root = root.clone();
            self.panes[side].active_mut().results = None;
            return self.load(side, root, None);
        }
        let cwd = t.panel.cwd().to_path_buf();
        if t.results.is_some() {
            self.say(StatusKind::Error, fl!("results-unsupported"));
            return Task::none();
        }
        let dirs: Vec<PathBuf> = if action == Action::BranchSel {
            // From the listing: inside an archive nothing is on disk.
            let dirs: std::collections::HashSet<&std::ffi::OsStr> = (t.panel.entries().iter())
                .filter(|e| e.is_dir())
                .map(|e| e.os_name.as_os_str())
                .collect();
            (t.panel.targets().into_iter())
                .filter(|p| p.file_name().is_some_and(|n| dirs.contains(n)))
                .collect()
        } else {
            vec![cwd.clone()]
        };
        if dirs.is_empty() {
            return Task::none();
        }
        let (tab, hidden) = (t.id, self.config.show_hidden);
        let id = self.next_id();
        let stop = Arc::new(AtomicBool::new(false));
        let t = self.panes[side].active_mut();
        t.branching = Some(id);
        t.branch_stop.store(true, Ordering::Relaxed); // an earlier read still going
        t.branch_stop = stop.clone();
        self.say(StatusKind::Busy, fl!("branch-reading"));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || match archive::split_path(&dirs[0]) {
                    // ponytail: the index is read whole, Esc does not stop it (as entering one).
                    Some((a, _)) => Some(
                        (dirs.iter())
                            .filter_map(|d| {
                                archive::branch(&a, d.strip_prefix(&a).ok()?, hidden).ok()
                            })
                            .flatten()
                            .collect(),
                    ),
                    None => listing::branch(&dirs, hidden, &stop),
                })
                .await
                .unwrap_or(Some(Vec::new())) // the walker died: an empty branch, as before
            },
            move |paths| {
                cosmic::Action::App(Message::BranchListed {
                    tab,
                    id,
                    root: cwd.clone(),
                    paths,
                })
            },
        )
    }
}
