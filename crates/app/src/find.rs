//! Alt+F7: runs `core::search::find` on a worker thread; results reach the UI in batches.

use cosmic::iced::futures::channel::mpsc as fmpsc;
use shagoff_core::search::{self, Query};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Every event carries the search id: a restarted search drops the old one's stragglers.
#[derive(Debug, Clone)]
pub enum FindEvent {
    Found(u64, Vec<PathBuf>),
    Dir(u64, PathBuf),
    Done(u64),
}

const EVERY: Duration = Duration::from_millis(100);

/// Matches go out at most every `EVERY`, and a pending one never waits for the next match:
/// the walk calls `tick` on every dir it enters.
struct Batcher {
    batch: Vec<PathBuf>,
    sent: Instant,
}

impl Batcher {
    fn new(now: Instant) -> Self {
        Self {
            batch: Vec::new(),
            sent: now,
        }
    }

    fn push(&mut self, p: PathBuf, now: Instant) -> Option<Vec<PathBuf>> {
        self.batch.push(p);
        self.tick(now)
    }

    fn tick(&mut self, now: Instant) -> Option<Vec<PathBuf>> {
        if self.batch.is_empty() || now.duration_since(self.sent) < EVERY {
            return None;
        }
        self.sent = now;
        Some(std::mem::take(&mut self.batch))
    }
}

pub fn spawn(
    id: u64,
    root: PathBuf,
    q: Query,
) -> (Arc<AtomicBool>, fmpsc::UnboundedReceiver<FindEvent>) {
    let (tx, rx) = fmpsc::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let b = RefCell::new(Batcher::new(Instant::now()));
        let mut dir_sent = Instant::now();
        let send = |batch: Option<Vec<PathBuf>>| {
            if let Some(v) = batch {
                let _ = tx.unbounded_send(FindEvent::Found(id, v));
            }
        };
        search::find(
            &root,
            &q,
            &flag,
            &mut |p| send(b.borrow_mut().push(p, Instant::now())),
            &mut |d| {
                send(b.borrow_mut().tick(Instant::now()));
                if dir_sent.elapsed() >= EVERY {
                    let _ = tx.unbounded_send(FindEvent::Dir(id, d.to_path_buf()));
                    dir_sent = Instant::now();
                }
            },
        );
        let rest = std::mem::take(&mut b.borrow_mut().batch);
        if !rest.is_empty() {
            let _ = tx.unbounded_send(FindEvent::Found(id, rest));
        }
        let _ = tx.unbounded_send(FindEvent::Done(id));
    });
    (stop, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::futures::{StreamExt, executor::block_on};
    use shagoff_core::mask::Mask;

    #[test]
    fn worker_streams_matches_then_done() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/a.rs"), "").unwrap();
        std::fs::write(d.path().join("b.txt"), "").unwrap();
        let q = Query {
            mask: Mask::parse("*.rs"),
            text: None,
            case_sensitive: false,
            hidden: false,
        };
        let (_stop, rx) = spawn(7, d.path().to_path_buf(), q);
        let events: Vec<FindEvent> = block_on(rx.collect());
        let found: Vec<PathBuf> = events
            .iter()
            .flat_map(|e| match e {
                FindEvent::Found(7, v) => v.clone(),
                _ => vec![],
            })
            .collect();
        assert_eq!(found, [d.path().join("sub/a.rs")]);
        assert!(matches!(events.last(), Some(FindEvent::Done(7))));
    }

    #[test]
    fn regression_pending_matches_flush_on_a_tick() {
        let t0 = Instant::now();
        let mut b = Batcher::new(t0);
        assert_eq!(b.push("/a".into(), t0 + Duration::from_millis(10)), None);
        // No further match for a long time: the walk's tick must still deliver "/a".
        assert_eq!(
            b.tick(t0 + Duration::from_millis(150)),
            Some(vec![PathBuf::from("/a")])
        );
        assert_eq!(b.tick(t0 + Duration::from_millis(400)), None);
    }
}
