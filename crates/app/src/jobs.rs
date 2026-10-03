//! Runs an `ops` job on a worker thread; events go to the UI over a channel, answers come back blocking.

use cosmic::iced::futures::channel::mpsc as fmpsc;
use shagoff_core::archive::{self, Format};
use shagoff_core::ops::{self, ErrorChoice, FileInfo, Handler, Method, Report, Resolution};
use shagoff_core::repack::{self, Change};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

pub enum Job {
    Transfer {
        method: Method,
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    Delete {
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    Pack {
        format: Format,
        base: PathBuf,
        groups: Vec<(Vec<PathBuf>, PathBuf)>,
        /// "Move to archive": delete the sources of archives written without skips.
        move_after: bool,
    },
    Unpack {
        archives: Vec<PathBuf>,
        dest: PathBuf,
        own_dir: bool,
    },
    /// Ctrl+Shift+S: copies by the dialog's arrows; existing targets ask as for F5.
    Sync {
        to_right: Vec<(PathBuf, PathBuf)>,
        to_left: Vec<(PathBuf, PathBuf)>,
    },
    /// F5 / Ctrl+C / Enter in an archive panel: `names` of the dir `inner` into `dest`.
    Extract {
        archive: PathBuf,
        inner: PathBuf,
        names: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// F6 out of an archive: extract, then delete from the archive what came out without skips.
    ExtractMove {
        archive: PathBuf,
        inner: PathBuf,
        names: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// F5 / F6 / F7 / F8 / Shift+F6 / F4 inside an archive: rewrite it with the change.
    Repack {
        archive: PathBuf,
        change: Change,
        /// F6 into the archive: delete the sources that were added completely.
        move_sources: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Event {
    Progress {
        done: u64,
        total: u64,
        current: PathBuf,
    },
    Conflict {
        src: FileInfo,
        dst: FileInfo,
        reply: mpsc::Sender<Resolution>,
    },
    Error {
        path: PathBuf,
        error: String,
        reply: mpsc::Sender<ErrorChoice>,
    },
    /// `Arc` because `Message` must be `Clone` and `Report` is not.
    Finished(Arc<Report>),
}

struct ChannelHandler {
    tx: fmpsc::UnboundedSender<Event>,
    cancel: Arc<AtomicBool>,
    last: Option<Instant>,
}

const PROGRESS_EVERY: Duration = Duration::from_millis(100);

impl Handler for ChannelHandler {
    fn progress(&mut self, done: u64, total: u64, current: &Path) {
        if self.last.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) && done < total {
            return;
        }
        self.last = Some(Instant::now());
        let _ = self.tx.unbounded_send(Event::Progress {
            done,
            total,
            current: current.to_path_buf(),
        });
    }

    fn conflict(&mut self, src: &FileInfo, dst: &FileInfo) -> Resolution {
        let (reply, rx) = mpsc::channel();
        let _ = self.tx.unbounded_send(Event::Conflict {
            src: src.clone(),
            dst: dst.clone(),
            reply,
        });
        rx.recv().unwrap_or(Resolution::Cancel) // UI dropped the sender → cancel, never hang
    }

    fn error(&mut self, path: &Path, err: &std::io::Error) -> ErrorChoice {
        let (reply, rx) = mpsc::channel();
        let _ = self.tx.unbounded_send(Event::Error {
            path: path.to_path_buf(),
            error: err.to_string(),
            reply,
        });
        rx.recv().unwrap_or(ErrorChoice::Cancel)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Start `job` on its own thread. Returns the cancel flag and the event stream (ends after `Finished`).
pub fn spawn(job: Job) -> (Arc<AtomicBool>, fmpsc::UnboundedReceiver<Event>) {
    let (tx, rx) = fmpsc::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut h = ChannelHandler {
        tx: tx.clone(),
        cancel: cancel.clone(),
        last: None,
    };
    std::thread::spawn(move || {
        let report = match job {
            Job::Transfer { method, pairs } => ops::transfer(method, &pairs, &mut h),
            Job::Delete { paths, permanent } => ops::delete(&paths, permanent, &mut h),
            Job::Pack {
                format,
                base,
                groups,
                move_after,
            } => {
                let r = archive::pack(format, &base, &groups, &mut h);
                if move_after && !r.cancelled && !r.completed.is_empty() {
                    let d = ops::delete(&r.completed, true, &mut h);
                    Report {
                        cancelled: d.cancelled,
                        completed: r.completed,
                    }
                } else {
                    r
                }
            }
            Job::Unpack {
                archives,
                dest,
                own_dir,
            } => archive::unpack(&archives, &dest, own_dir, &mut h),
            Job::Sync { to_right, to_left } => {
                let a = ops::transfer(Method::Copy, &to_right, &mut h);
                if a.cancelled {
                    a
                } else {
                    let b = ops::transfer(Method::Copy, &to_left, &mut h);
                    Report {
                        cancelled: b.cancelled,
                        completed: a.completed.into_iter().chain(b.completed).collect(),
                    }
                }
            }
            Job::Extract {
                archive,
                inner,
                names,
                dest,
            } => archive::extract(&archive, &inner, &names, &dest, &mut h),
            Job::ExtractMove {
                archive,
                inner,
                names,
                dest,
            } => {
                let r = archive::extract(&archive, &inner, &names, &dest, &mut h);
                let entries: Vec<PathBuf> = r
                    .completed
                    .iter()
                    .filter_map(|p| p.strip_prefix(&archive).ok().map(Path::to_path_buf))
                    .collect();
                if r.cancelled || entries.is_empty() {
                    r
                } else {
                    let d = repack::modify(&archive, &Change::Delete(entries), &mut h);
                    Report {
                        cancelled: d.cancelled,
                        completed: d.completed,
                    }
                }
            }
            Job::Repack {
                archive,
                change,
                move_sources,
            } => {
                let r = repack::modify(&archive, &change, &mut h);
                if move_sources && !r.cancelled && !r.completed.is_empty() {
                    let d = ops::delete(&r.completed, true, &mut h);
                    Report {
                        cancelled: d.cancelled,
                        completed: r.completed,
                    }
                } else {
                    r
                }
            }
        };
        let _ = tx.unbounded_send(Event::Finished(Arc::new(report)));
    });
    (cancel, rx)
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use cosmic::iced::futures::{StreamExt, executor::block_on};

    #[test]
    fn regression_sync_asks_before_replacing_and_deletes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (l, r) = (d.path().join("l"), d.path().join("r"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("a"), "new").unwrap();
        std::fs::write(r.join("a"), "old").unwrap();
        std::fs::write(r.join("extra"), "keep").unwrap();
        let job = Job::Sync {
            to_right: vec![(l.join("a"), r.join("a"))],
            to_left: vec![],
        };
        let (_cancel, mut rx) = spawn(job);
        let asked = block_on(async {
            let mut asked = false;
            while let Some(e) = rx.next().await {
                if let Event::Conflict { reply, .. } = e {
                    asked = true;
                    let _ = reply.send(Resolution::Skip);
                }
            }
            asked
        });
        assert!(asked);
        assert_eq!(std::fs::read_to_string(r.join("a")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(r.join("extra")).unwrap(), "keep");
    }

    /// Run a job to the end, answering nothing (no conflicts or errors expected).
    fn run(job: Job) -> Arc<Report> {
        let (_cancel, mut rx) = spawn(job);
        block_on(async {
            while let Some(e) = rx.next().await {
                if let Event::Finished(r) = e {
                    return r;
                }
            }
            panic!("no Finished")
        })
    }

    #[test]
    fn move_into_and_out_of_an_archive() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("f.txt");
        std::fs::write(&src, "F").unwrap();
        let a = d.path().join("a.zip");
        let r = archive::pack(
            Format::Zip,
            d.path(),
            &[(vec![src.clone()], a.clone())],
            &mut NoAsk,
        );
        assert!(!r.cancelled);
        std::fs::write(d.path().join("g.txt"), "G").unwrap();
        // F6 into the archive: g.txt goes in, the source is gone.
        run(Job::Repack {
            archive: a.clone(),
            change: Change::Add {
                sources: vec![d.path().join("g.txt")],
                inner: "".into(),
            },
            move_sources: true,
        });
        assert!(!d.path().join("g.txt").exists());
        // F6 out of it: f.txt comes out and leaves the archive.
        std::fs::remove_file(&src).unwrap();
        let out = d.path().join("out");
        run(Job::ExtractMove {
            archive: a.clone(),
            inner: "".into(),
            names: vec!["f.txt".into()],
            dest: out.clone(),
        });
        assert_eq!(std::fs::read_to_string(out.join("f.txt")).unwrap(), "F");
        let names: Vec<String> = archive::list(&a, Path::new(""), true)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["g.txt"]);
    }

    struct NoAsk;
    impl Handler for NoAsk {
        fn progress(&mut self, _: u64, _: u64, _: &Path) {}
        fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
            Resolution::Cancel
        }
        fn error(&mut self, _: &Path, _: &std::io::Error) -> ErrorChoice {
            ErrorChoice::Cancel
        }
        fn cancelled(&self) -> bool {
            false
        }
    }
}
