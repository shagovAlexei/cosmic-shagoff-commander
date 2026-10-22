//! The running job: start, queue, cancel, its events and its end.

use super::*;

impl App {
    /// `action` would change an archive opened from inside another (155): a temp copy.
    pub(super) fn nested_change(&self, side: usize, action: Action) -> bool {
        let nested = |s: usize| {
            let t = self.panes[s].active().target();
            t.starts_with(archive::temp_root()) && archive::split_path(&t).is_some()
        };
        use Action as A;
        let here = matches!(
            action,
            A::Delete
                | A::DeletePermanent
                | A::Mkdir
                | A::Rename
                | A::NewFile
                | A::MultiRename
                | A::Edit
                | A::Move
                | A::CopySame
                | A::Properties
                | A::ClipCut
                | A::ClipPaste
        );
        (here && nested(side)) || (matches!(action, A::Copy | A::Move) && nested(1 - side))
    }

    /// Actions that would change an archive (this panel inside one, or the other one as target).
    pub(super) fn read_only(&self, side: usize, action: Action) -> bool {
        let inside = |s: usize| archive::split_path(&self.panes[s].active().target()).is_some();
        // F4, F7, F8, Shift+F6 inside and F5 / F6 into or out of one rewrite the archive.
        match action {
            // Shift+F5 inside one copies from the archive into itself.
            Action::CopySame | Action::NewFile | Action::NewFromTemplate => inside(side),
            // A link to or into an archive's inside would point at nothing on disk.
            Action::Pack | Action::MoveToArchive | Action::Unpack | Action::Link => {
                inside(side) || inside(1 - side)
            }
            Action::ChecksumCreate => inside(side),
            // Parts are read and written as files on disk.
            Action::SplitFile | Action::CombineFiles => inside(side) || inside(1 - side),
            // One side in an archive syncs (158): jobs extract from it and add to it.
            Action::SyncDirs => inside(side) && inside(1 - side),
            _ => false,
        }
    }

    pub(super) fn start_delete(
        &mut self,
        side: usize,
        permanent: bool,
        paths: Vec<PathBuf>,
    ) -> Task<Message> {
        // Search results in the panel: each archive rewritten once, the files on disk together.
        let groups = archive::split_by(&paths, |p| {
            p.parent().and_then(archive::split_path).map(|(a, _)| a)
        });
        if groups.len() > 1 {
            let tasks: Vec<_> = (groups.into_iter())
                .map(|g| self.start_delete(side, permanent, g))
                .collect();
            return Task::batch(tasks);
        }
        let dir = paths
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        if let Some((archive, _)) = dir.and_then(|d| archive::split_path(&d)) {
            let entries = paths
                .iter()
                .filter_map(|p| p.strip_prefix(&archive).ok().map(Path::to_path_buf))
                .collect();
            let job = Job::Repack {
                archive,
                change: Change::Delete(entries),
            };
            return self.start_job(side, OpKind::Repack, job, None);
        }
        self.start_job(side, OpKind::Delete, Job::Delete { paths, permanent }, None)
    }

    pub(super) fn start_job(
        &mut self,
        side: usize,
        kind: OpKind,
        job: Job,
        focus: Option<String>,
    ) -> Task<Message> {
        self.start_queued(side, kind, job, focus, None)
    }

    pub(super) fn start_queued(
        &mut self,
        side: usize,
        kind: OpKind,
        job: Job,
        focus: Option<String>,
        replan: Option<(Vec<PathBuf>, PathBuf)>,
    ) -> Task<Message> {
        if changes_nested(&job) {
            self.say(StatusKind::Error, fl!("nested-read-only"));
            return Task::none();
        }
        // One at a time: a second one waits in the queue for the first to end (TC F2 «В очередь»).
        if self.job.is_some() {
            self.queue.push_back(Queued {
                side,
                kind,
                job,
                focus,
                replan,
            });
            self.say(StatusKind::Info, fl!("job-queued", n = self.queue.len()));
            return Task::none();
        }
        let (control, events) = jobs::spawn(job);
        self.job = Some(Running {
            side,
            kind,
            done: 0,
            total: 0,
            current: String::new(),
            control,
            focus,
            open: None,
            hidden: false,
        });
        Task::run(events, |e| cosmic::Action::App(Message::Op(e)))
    }

    /// Cancel stops what waits too: a queued delete must not start on its own right after.
    pub(super) fn cancel_job(&mut self) {
        if let Some(j) = &self.job {
            j.control.cancel.store(true, Ordering::Relaxed);
        }
        self.queue.clear();
        self.held = None; // dropping the reply answers Cancel
    }

    pub(super) fn on_job_event(&mut self, event: jobs::Event) -> Task<Message> {
        match event {
            jobs::Event::Progress {
                done,
                total,
                current,
            } => {
                if let Some(j) = &mut self.job {
                    (j.done, j.total) = (done, total);
                    j.current = current
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
            }
            jobs::Event::Conflict { src, dst, reply } => {
                let name = ops::unique_name(&dst.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.ask(Dialog::Conflict {
                    src,
                    dst,
                    reply,
                    name,
                });
            }
            jobs::Event::Error { path, error, reply } if archive::needs_password(&error) => {
                let wrong = archive::password(&path).is_some();
                self.ask(Dialog::Password {
                    archive: path,
                    password: String::new(),
                    wrong,
                    reply: Some(reply),
                    open: None,
                });
                return widget::text_input::focus(self.input_id.clone());
            }
            jobs::Event::Error { path, error, reply } => {
                self.ask(Dialog::Error { path, error, reply });
            }
            jobs::Event::Finished(report) => return self.finish_job(&report),
        }
        Task::none()
    }

    /// The job waits for the answer: an open dialog is not replaced, the question comes after it.
    pub(super) fn ask(&mut self, d: Dialog) {
        if self.dialog.is_some() {
            self.held = Some(d);
            self.say(StatusKind::Info, fl!("job-waits-answer"));
        } else {
            self.dialog = Some(d);
        }
    }

    /// Unmark what was processed and reload both panes (cursor on the renamed entry, if any).
    pub(super) fn finish_job(&mut self, report: &Report) -> Task<Message> {
        let Some(job) = self.job.take() else {
            return Task::none();
        };
        // Not put back (an error, cancelled): the next tick asks about that change again.
        if let Some(file) = self.updating.take()
            && (report.cancelled || report.completed.is_empty())
            && let Some(e) = self.edited.iter_mut().find(|e| e.file == file)
        {
            e.mtime = SystemTime::UNIX_EPOCH;
        }
        if self.quitting {
            return self.handle(Message::Exit);
        }
        let side = job.side;
        let mut view = Task::none();
        let mut browse = None;
        let mut focus = job.focus;
        // N / P in the viewer inside an archive, its extraction cancelled or failed: the cursor
        // goes back to the file still on screen.
        if let Some((Open::Lister(_), file)) = &job.open
            && (report.cancelled || !file.exists())
            && let Some(name) = (self.lister.as_ref())
                .filter(|l| l.side == side)
                .map(|l| l.name.clone())
        {
            let panel = &mut self.panes[side].active_mut().panel;
            if let Some(i) = panel.entries().iter().position(|e| e.name == name) {
                panel.set_cursor(i);
            }
            focus = Some(name);
        }
        if let Some((open, file)) = job.open
            && !report.cancelled
            && file.exists()
        {
            match open {
                Open::Run(argv) => {
                    if let Err(err) = spawn_detached(&argv) {
                        self.say(
                            StatusKind::Error,
                            fl!("open-failed", err = crate::why::text(&err)),
                        );
                    }
                }
                Open::Edit {
                    argv,
                    archive,
                    entry,
                } => match spawn_detached(&argv) {
                    Ok(()) => {
                        let mtime = std::fs::symlink_metadata(&file)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        self.edited.push(Edited {
                            file,
                            archive,
                            entry,
                            mtime,
                        });
                    }
                    Err(err) => self.say(
                        StatusKind::Error,
                        fl!("open-failed", err = crate::why::text(&err)),
                    ),
                },
                Open::Lister(name) => view = self.open_lister(side, file, name),
                Open::Browse(back) => {
                    self.nested.insert(file.clone(), back);
                    browse = Some(file);
                }
            }
        }
        self.panes[side]
            .active_mut()
            .panel
            .unmark(&report.completed);
        // A tab still loading a dir is left to that read (reading it again would drop the cursor
        // Backspace asked for); the others are read again.
        let again = |app: &mut Self, s: usize, focus: Option<String>| {
            let t = app.panes[s].active();
            match t.pending {
                Some(_) => Task::none(),
                None => {
                    let cwd = t.panel.cwd().to_path_buf();
                    app.reload(s, cwd, focus)
                }
            }
        };
        let reload = Task::batch([
            match browse {
                Some(arc) => self.load(side, arc, None),
                None => again(self, side, focus),
            },
            again(self, 1 - side, None),
            view,
        ]);
        // The next one in the background, as TC's queue.
        while let Some(mut q) = self.queue.pop_front() {
            if let (Some((sources, target)), Job::Transfer { pairs, .. }) = (&q.replan, &mut q.job)
            {
                match ops::plan(sources, target) {
                    Ok(p) => *pairs = p,
                    Err(e) => {
                        // Under the error already shown (the job just ended, one dropped before).
                        let text = match self.status.take() {
                            Some(s) if s.kind == StatusKind::Error => {
                                format!("{}\n{}", s.text, plan_error(&e))
                            }
                            _ => plan_error(&e),
                        };
                        self.say(StatusKind::Error, text);
                        continue;
                    }
                }
            }
            let next = self.start_job(q.side, q.kind, q.job, q.focus);
            if let Some(j) = &mut self.job {
                j.hidden = true;
            }
            return Task::batch([reload, next]);
        }
        reload
    }
}
