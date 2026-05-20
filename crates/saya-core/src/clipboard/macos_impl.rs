use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};

use crate::Database;

#[cfg(feature = "embedding")]
use crate::ai::EmbedderHandle;

pub const POLL_INTERVAL: Duration = Duration::from_millis(300);
pub const MAX_BYTES: usize = 100 * 1024;

/// Callback invoked after a clipboard entry is successfully committed to the
/// database. Args: (entry_id, content, created_at_unix_ms). The closure must
/// be cheap or dispatch its own background work — it runs on the monitor's
/// polling thread.
pub type OnInsert = Arc<dyn Fn(i64, String, i64) + Send + Sync + 'static>;

pub struct ClipboardMonitor {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    on_insert: Arc<Mutex<Option<OnInsert>>>,
}

impl ClipboardMonitor {
    pub fn start(db: Database) -> Self {
        Self::spawn(db, None)
    }

    #[cfg(feature = "embedding")]
    pub fn start_with_embedder(db: Database, embedder: EmbedderHandle) -> Self {
        Self::spawn(db, Some(embedder))
    }

    /// Attach (or detach with `None`) an observer to be notified after each
    /// committed entry. Live-changeable while the monitor is running.
    pub fn set_on_insert(&self, cb: Option<OnInsert>) {
        *self.on_insert.lock().expect("on_insert lock") = cb;
    }

    fn spawn(db: Database, embedder: OptEmbedder) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let on_insert: Arc<Mutex<Option<OnInsert>>> = Arc::new(Mutex::new(None));
        let running_thread = running.clone();
        let on_insert_thread = on_insert.clone();
        let handle = std::thread::Builder::new()
            .name("saya-clipboard".into())
            .spawn(move || run_loop(db, embedder, on_insert_thread, running_thread))
            .expect("spawn clipboard thread");
        Self {
            running,
            handle: Some(handle),
            on_insert,
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ClipboardMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(feature = "embedding")]
type OptEmbedder = Option<EmbedderHandle>;
#[cfg(not(feature = "embedding"))]
type OptEmbedder = Option<std::convert::Infallible>;

fn run_loop(
    db: Database,
    embedder: OptEmbedder,
    on_insert: Arc<Mutex<Option<OnInsert>>>,
    running: Arc<AtomicBool>,
) {
    let pasteboard = NSPasteboard::generalPasteboard();
    let mut last_change = pasteboard.changeCount();

    while running.load(Ordering::Relaxed) {
        std::thread::sleep(POLL_INTERVAL);
        if !running.load(Ordering::Relaxed) {
            break;
        }
        let current = pasteboard.changeCount();
        if current == last_change {
            continue;
        }
        last_change = current;

        let Some(text) = read_text(&pasteboard) else { continue };
        if text.is_empty() {
            continue;
        }
        if text.len() > MAX_BYTES {
            tracing::debug!(len = text.len(), "clipboard text exceeds MAX_BYTES, skipped");
            continue;
        }

        let inserted = match db.insert_entry(&text) {
            Ok(Some(id)) => {
                tracing::debug!(id, len = text.len(), "clipboard captured");
                // Notify observer with a snapshot copy so it can hold/forward
                // the content without back-pressuring this thread.
                let cb = on_insert.lock().expect("on_insert lock").clone();
                if let Some(cb) = cb {
                    cb(id, text.clone(), unix_ms());
                }
                Some(id)
            }
            Ok(None) => {
                tracing::trace!("clipboard duplicate, skipped");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "clipboard insert failed");
                None
            }
        };

        #[cfg(feature = "embedding")]
        if let (Some(id), Some(emb)) = (inserted, embedder.as_ref()) {
            match emb.embed_one(&text) {
                Ok(v) => {
                    if let Err(e) = db.upsert_vector(id, &v) {
                        tracing::warn!(error = %e, id, "vector upsert failed");
                    }
                }
                Err(e) => tracing::warn!(error = %e, id, "embed failed"),
            }
        }

        let _ = inserted;
        let _ = &embedder;
    }
}

fn read_text(pb: &NSPasteboard) -> Option<String> {
    // SAFETY: passing the AppKit-provided NSPasteboardTypeString global.
    let ns = unsafe { pb.stringForType(NSPasteboardTypeString) }?;
    Some(ns.to_string())
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
