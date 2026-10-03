//! Alt+F7: runs `core::search::find` on a worker thread; results reach the UI in batches.

use cosmic::iced::futures::channel::mpsc as fmpsc;
use shagoff_core::search::{self, Query};
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

pub fn spawn(
    id: u64,
    root: PathBuf,
    q: Query,
) -> (Arc<AtomicBool>, fmpsc::UnboundedReceiver<FindEvent>) {
    let (tx, rx) = fmpsc::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let (mut batch, mut sent, mut dir_sent) = (Vec::new(), Instant::now(), Instant::now());
        let dtx = tx.clone();
        search::find(
            &root,
            &q,
            &flag,
            &mut |p| {
                batch.push(p);
                if sent.elapsed() >= EVERY {
                    let _ = tx.unbounded_send(FindEvent::Found(id, std::mem::take(&mut batch)));
                    sent = Instant::now();
                }
            },
            &mut |d| {
                if dir_sent.elapsed() >= EVERY {
                    let _ = dtx.unbounded_send(FindEvent::Dir(id, d.to_path_buf()));
                    dir_sent = Instant::now();
                }
            },
        );
        if !batch.is_empty() {
            let _ = tx.unbounded_send(FindEvent::Found(id, batch));
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
}
