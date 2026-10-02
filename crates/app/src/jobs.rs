//! Runs an `ops` job on a worker thread; events go to the UI over a channel, answers come back blocking.

use cosmic::iced::futures::channel::mpsc as fmpsc;
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
        };
        let _ = tx.unbounded_send(Event::Finished(Arc::new(report)));
    });
    (cancel, rx)
}
