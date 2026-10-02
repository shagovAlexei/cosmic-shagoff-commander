//! Auto-refresh: a notify watch on one pane's cwd, debounced into `Message::Changed(side)`.

use crate::app::Message;
use cosmic::iced::Subscription;
use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Quiet time before a rescan, and the longest a stream of events can delay it.
const QUIET: Duration = Duration::from_millis(200);
const MAX_DELAY: Duration = Duration::from_secs(1);

/// Opening or reading is not a change; closing after a write is.
pub fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Recreated whenever `(side, dir)` changes (iced keys subscriptions by their data).
pub fn watch(side: usize, dir: PathBuf) -> Subscription<Message> {
    Subscription::run_with((side, dir), |(side, dir)| {
        let (side, dir) = (*side, dir.clone());
        cosmic::iced::stream::channel(1, async move |mut out| {
            let (tx, mut rx) = mpsc::unbounded();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok_and(|e| relevant(&e.kind)) {
                    let _ = tx.unbounded_send(());
                }
            })
            .and_then(|mut w| w.watch(&dir, RecursiveMode::NonRecursive).map(|()| w));
            // Kept alive for the life of the stream; dropping it stops the watch.
            let _watcher = match watcher {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("watch {}: {e}", dir.display());
                    return std::future::pending().await;
                }
            };
            while rx.next().await.is_some() {
                let start = Instant::now();
                while start.elapsed() < MAX_DELAY {
                    match tokio::time::timeout(QUIET, rx.next()).await {
                        Ok(Some(())) => continue,
                        _ => break,
                    }
                }
                if out.send(Message::Changed(side)).await.is_err() {
                    break;
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};

    #[test]
    fn open_and_read_events_are_ignored() {
        // Our own scan opens the dir (IN_OPEN): reacting to it would rescan forever.
        assert!(!relevant(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!relevant(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!relevant(&EventKind::Access(AccessKind::Read)));
    }

    #[test]
    fn changes_are_relevant() {
        assert!(relevant(&EventKind::Create(CreateKind::File)));
        assert!(relevant(&EventKind::Remove(RemoveKind::Any)));
        assert!(relevant(&EventKind::Modify(ModifyKind::Any)));
        assert!(relevant(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
    }
}
