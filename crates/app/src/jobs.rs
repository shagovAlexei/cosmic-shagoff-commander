//! Runs an `ops` job on a worker thread; events go to the UI over a channel, answers come back blocking.

use cosmic::iced::futures::channel::mpsc as fmpsc;
use shagoff_core::archive::{self, Format};
use shagoff_core::ops::{self, ErrorChoice, FileInfo, Handler, Method, Report, Resolution};
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
    /// Ctrl+Shift+S: copies by the dialog's arrows; an older target is replaced without asking.
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

/// Sync: the direction was chosen in the dialog, so a conflict means "replace".
struct Replacing<'a>(&'a mut dyn Handler);

impl Handler for Replacing<'_> {
    fn progress(&mut self, done: u64, total: u64, current: &Path) {
        self.0.progress(done, total, current);
    }
    fn conflict(&mut self, _: &FileInfo, _: &FileInfo) -> Resolution {
        Resolution::Replace
    }
    fn error(&mut self, path: &Path, err: &std::io::Error) -> ErrorChoice {
        self.0.error(path, err)
    }
    fn cancelled(&self) -> bool {
        self.0.cancelled()
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
                let mut h = Replacing(&mut h);
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
    fn sync_replaces_without_asking_and_deletes_nothing() {
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
        let (_cancel, rx) = spawn(job);
        let events: Vec<Event> = block_on(rx.collect());
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, Event::Conflict { .. } | Event::Error { .. }))
        );
        assert_eq!(std::fs::read_to_string(r.join("a")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(r.join("extra")).unwrap(), "keep");
    }
}
