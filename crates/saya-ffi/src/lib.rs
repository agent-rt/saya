//! UniFFI surface consumed by the SwiftUI shell.
//!
//! Design notes:
//! - Single opaque `Saya` object owns all state (DB, lazy launcher index,
//!   clipboard monitor, optional embedder). Swift sees a thin RPC-style API.
//! - DTO types mirror saya-core records but use FFI-safe field types
//!   (`String` for paths, `i64` for unix-ms timestamps).
//! - Feature-gated paths (`embedding`) keep stable signatures and return a
//!   runtime `Internal` error when the feature wasn't compiled in. This keeps
//!   the generated Swift bindings stable regardless of build flavor.

uniffi::setup_scaffolding!("saya");

mod logging;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use saya_core::clipboard;
use saya_core::database::Database;
use saya_core::launcher::{self, LauncherIndex};
use saya_core::search::{SearchQuery, Searcher};

#[cfg(feature = "embedding")]
use saya_core::ai::EmbedderHandle;

// ---- DTOs ----------------------------------------------------------------

#[derive(Debug, Clone, uniffi::Record)]
pub struct ClipboardEntryDto {
    pub id: i64,
    pub content: String,
    pub byte_size: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SearchHitDto {
    pub id: i64,
    pub content: String,
    pub score: f32,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AppEntryDto {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct MatchedAppDto {
    pub name: String,
    pub path: String,
    pub score: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct StatusDto {
    pub db_path: String,
    pub entry_count: u64,
    pub entries_missing_vectors: u64,
    pub clipboard_monitor_running: bool,
    pub embedder_loaded: bool,
    pub embedding_feature_compiled: bool,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SayaError {
    #[error("{0}")]
    Internal(String),
}

/// Implemented by the Swift app to receive realtime clipboard events.
/// Called from the monitor's polling thread; the foreign implementation
/// should hop to its preferred queue/actor before mutating UI state.
#[uniffi::export(with_foreign)]
pub trait ClipboardObserver: Send + Sync {
    fn on_entry_captured(&self, entry: ClipboardEntryDto);
}

impl From<saya_core::Error> for SayaError {
    fn from(e: saya_core::Error) -> Self {
        SayaError::Internal(e.to_string())
    }
}

// ---- Conversions ---------------------------------------------------------

impl From<saya_core::Entry> for ClipboardEntryDto {
    fn from(e: saya_core::Entry) -> Self {
        Self {
            id: e.id,
            content: e.content,
            byte_size: e.byte_size,
            created_at_unix_ms: e.created_at,
        }
    }
}

impl From<saya_core::SearchHit> for SearchHitDto {
    fn from(h: saya_core::SearchHit) -> Self {
        Self {
            id: h.id,
            content: h.content,
            score: h.score,
            created_at_unix_ms: h.created_at,
        }
    }
}

impl From<launcher::AppEntry> for AppEntryDto {
    fn from(a: launcher::AppEntry) -> Self {
        Self {
            name: a.name,
            path: a.path.to_string_lossy().into_owned(),
        }
    }
}

impl From<launcher::MatchedApp> for MatchedAppDto {
    fn from(m: launcher::MatchedApp) -> Self {
        Self {
            name: m.app.name,
            path: m.app.path.to_string_lossy().into_owned(),
            score: m.score,
        }
    }
}

// ---- Saya object ---------------------------------------------------------

#[derive(uniffi::Object)]
pub struct Saya {
    db: Database,
    db_path: PathBuf,
    launcher: Mutex<Option<Arc<LauncherIndex>>>,
    monitor: Mutex<Option<clipboard::ClipboardMonitor>>,
    // Cached snapshot of the app launch history. Invalidated whenever
    // `launch_app` records a new launch. Avoids a SQLite hit on every
    // launcher keystroke.
    mru_cache: Mutex<Option<std::collections::HashMap<String, saya_core::database::MruInfo>>>,
    clipboard_observer: Mutex<Option<Arc<dyn ClipboardObserver>>>,
    #[cfg(feature = "embedding")]
    embedder: Mutex<Option<EmbedderHandle>>,
}

#[uniffi::export]
impl Saya {
    /// Open (or create) the saya database at `db_path`. The directory is
    /// created if missing.
    #[uniffi::constructor]
    pub fn new(db_path: String) -> Result<Arc<Self>, SayaError> {
        // Idempotent — the first Saya instance wires up tracing for the
        // whole process; subsequent calls (e.g. tests, CLI) are no-ops.
        logging::init();
        let path = PathBuf::from(&db_path);
        tracing::info!(db_path = %path.display(), "opening Saya");
        let db = Database::open(&path).inspect_err(|e| {
            tracing::error!(error = %e, "database open failed");
        })?;
        tracing::info!("Saya ready");
        Ok(Arc::new(Self {
            db,
            db_path: path,
            launcher: Mutex::new(None),
            monitor: Mutex::new(None),
            mru_cache: Mutex::new(None),
            clipboard_observer: Mutex::new(None),
            #[cfg(feature = "embedding")]
            embedder: Mutex::new(None),
        }))
    }

    /// Hybrid clipboard search. `semantic = true` engages the vector lane,
    /// which lazy-loads the embedder. Falls back to literal-only on failure.
    pub fn search(
        &self,
        query: String,
        limit: u32,
        semantic: bool,
    ) -> Result<Vec<SearchHitDto>, SayaError> {
        #[allow(unused_mut)]
        let mut searcher = Searcher::new(self.db.clone());
        if semantic {
            #[cfg(feature = "embedding")]
            {
                searcher = searcher.with_embedder(self.ensure_embedder());
            }
            #[cfg(not(feature = "embedding"))]
            {
                return Err(SayaError::Internal(
                    "semantic search requires the `embedding` feature".into(),
                ));
            }
        }
        let hits = searcher.search(&SearchQuery {
            text: query,
            limit: limit as usize,
        })?;
        Ok(hits.into_iter().map(Into::into).collect())
    }

    /// Return all installed apps (cached after first call).
    pub fn apps(&self) -> Result<Vec<AppEntryDto>, SayaError> {
        let idx = self.ensure_launcher()?;
        Ok(idx.apps().iter().cloned().map(Into::into).collect())
    }

    pub fn match_apps(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<MatchedAppDto>, SayaError> {
        let started = std::time::Instant::now();
        let idx = self.ensure_launcher()?;
        let mru = self.cached_mru()?;
        let results: Vec<MatchedAppDto> = idx
            .match_query(&query, limit as usize, &mru)
            .into_iter()
            .map(Into::into)
            .collect();
        tracing::debug!(
            query = %query,
            limit,
            results = results.len(),
            elapsed = ?started.elapsed(),
            "match_apps"
        );
        Ok(results)
    }

    /// Rebuild the app index from disk (e.g. after the user installs a new app).
    pub fn refresh_apps(&self) -> Result<u32, SayaError> {
        let idx = LauncherIndex::build()?;
        let n = idx.apps().len() as u32;
        *self.launcher.lock().expect("launcher lock") = Some(Arc::new(idx));
        Ok(n)
    }

    /// Warm the on-disk icon cache in the background. Returns immediately;
    /// the prefetch thread runs to completion regardless of further calls.
    pub fn prefetch_icons(&self) -> Result<(), SayaError> {
        let idx = self.ensure_launcher()?;
        idx.prefetch_icons();
        Ok(())
    }

    pub fn launch_app(&self, path: String) -> Result<(), SayaError> {
        launcher::launch(Path::new(&path))?;
        if let Err(e) = self.db.record_launch(&path) {
            tracing::warn!(error = %e, path = %path, "record_launch failed");
        }
        // Invalidate the cached MRU snapshot so the next match call sees
        // the bump.
        *self.mru_cache.lock().expect("mru cache lock") = None;
        Ok(())
    }

    pub fn icon_png(&self, path: String) -> Result<Vec<u8>, SayaError> {
        launcher::icon_png(Path::new(&path)).map_err(Into::into)
    }

    /// Manually insert a clipboard entry (e.g. from a Swift-side test or import).
    /// Returns the new id, or null if it was a consecutive duplicate.
    pub fn insert_clipboard_entry(&self, text: String) -> Result<Option<i64>, SayaError> {
        Ok(self.db.insert_entry(&text)?)
    }

    pub fn recent_clipboard(&self, limit: u32) -> Result<Vec<ClipboardEntryDto>, SayaError> {
        Ok(self
            .db
            .recent(limit as usize)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Start the clipboard monitor. `embed = true` also feeds each new entry
    /// into the embedder; requires the `embedding` feature.
    pub fn start_clipboard_monitor(&self, embed: bool) -> Result<(), SayaError> {
        let mut guard = self.monitor.lock().expect("monitor lock");
        if guard.is_some() {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            let mon = if embed {
                #[cfg(feature = "embedding")]
                {
                    let e = self.ensure_embedder();
                    clipboard::ClipboardMonitor::start_with_embedder(self.db.clone(), e)
                }
                #[cfg(not(feature = "embedding"))]
                {
                    return Err(SayaError::Internal(
                        "embed=true requires the `embedding` feature".into(),
                    ));
                }
            } else {
                clipboard::ClipboardMonitor::start(self.db.clone())
            };
            // Wire the observer (if one was registered before the monitor
            // started) into the freshly spawned monitor.
            if let Some(obs) = self
                .clipboard_observer
                .lock()
                .expect("observer lock")
                .clone()
            {
                mon.set_on_insert(Some(Self::make_on_insert(obs)));
            }
            *guard = Some(mon);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = embed;
            Err(SayaError::Internal(
                "clipboard monitor is macOS-only".into(),
            ))
        }
    }

    /// Register (or clear with `None`) a callback to receive every newly
    /// captured clipboard entry. Live-changeable: takes effect immediately,
    /// even while the monitor is running.
    pub fn set_clipboard_observer(&self, observer: Option<Arc<dyn ClipboardObserver>>) {
        *self.clipboard_observer.lock().expect("observer lock") = observer.clone();
        #[cfg(target_os = "macos")]
        if let Some(mon) = self.monitor.lock().expect("monitor lock").as_ref() {
            mon.set_on_insert(observer.map(Self::make_on_insert));
        }
    }

    pub fn stop_clipboard_monitor(&self) {
        if let Some(mut m) = self.monitor.lock().expect("monitor lock").take() {
            m.stop();
        }
    }

    /// Backfill embeddings for entries that don't yet have a vector.
    /// Returns the number processed.
    pub fn reindex(&self, limit: u32, batch: u32) -> Result<u32, SayaError> {
        #[cfg(feature = "embedding")]
        {
            let emb = self.ensure_embedder();
            let pending = self.db.entries_missing_vectors(limit as usize)?;
            let total = pending.len() as u32;
            let bs = batch.max(1) as usize;
            for chunk in pending.chunks(bs) {
                let texts: Vec<&str> = chunk.iter().map(|e| e.content.as_str()).collect();
                let vecs = emb.embed(&texts)?;
                for (entry, v) in chunk.iter().zip(vecs.iter()) {
                    self.db.upsert_vector(entry.id, v)?;
                }
            }
            Ok(total)
        }
        #[cfg(not(feature = "embedding"))]
        {
            let _ = (limit, batch);
            Err(SayaError::Internal(
                "reindex requires the `embedding` feature".into(),
            ))
        }
    }

    /// Drop the in-memory embedder model, releasing Metal context. Safe to
    /// call at any time; next semantic op will reload lazily.
    pub fn unload_embedder(&self) {
        #[cfg(feature = "embedding")]
        if let Some(e) = self.embedder.lock().expect("embedder lock").as_ref() {
            e.unload();
        }
    }

    pub fn status(&self) -> Result<StatusDto, SayaError> {
        let entry_count = self.db.count()? as u64;
        let entries_missing_vectors =
            self.db.entries_missing_vectors(i64::MAX as usize)?.len() as u64;
        let clipboard_monitor_running =
            self.monitor.lock().expect("monitor lock").is_some();
        #[cfg(feature = "embedding")]
        let embedder_loaded = self
            .embedder
            .lock()
            .expect("embedder lock")
            .as_ref()
            .is_some_and(|e| e.is_loaded());
        #[cfg(not(feature = "embedding"))]
        let embedder_loaded = false;
        Ok(StatusDto {
            db_path: self.db_path.to_string_lossy().into_owned(),
            entry_count,
            entries_missing_vectors,
            clipboard_monitor_running,
            embedder_loaded,
            embedding_feature_compiled: cfg!(feature = "embedding"),
        })
    }
}

// ---- internal helpers (not exported) -------------------------------------

impl Saya {
    fn ensure_launcher(&self) -> Result<Arc<LauncherIndex>, SayaError> {
        let mut guard = self.launcher.lock().expect("launcher lock");
        if guard.is_none() {
            *guard = Some(Arc::new(LauncherIndex::build()?));
        }
        Ok(guard.as_ref().unwrap().clone())
    }

    #[cfg(target_os = "macos")]
    fn make_on_insert(obs: Arc<dyn ClipboardObserver>) -> clipboard::OnInsert {
        Arc::new(move |id, content, ts| {
            let byte_size = content.len() as i64;
            obs.on_entry_captured(ClipboardEntryDto {
                id,
                content,
                byte_size,
                created_at_unix_ms: ts,
            });
        })
    }

    fn cached_mru(
        &self,
    ) -> Result<std::collections::HashMap<String, saya_core::database::MruInfo>, SayaError> {
        let mut guard = self.mru_cache.lock().expect("mru cache lock");
        if guard.is_none() {
            *guard = Some(self.db.launch_history()?);
        }
        Ok(guard.as_ref().expect("mru cache").clone())
    }

    #[cfg(feature = "embedding")]
    fn ensure_embedder(&self) -> EmbedderHandle {
        let mut guard = self.embedder.lock().expect("embedder lock");
        if guard.is_none() {
            *guard = Some(EmbedderHandle::new());
        }
        guard.as_ref().unwrap().clone()
    }
}

/// Default DB path (`~/Library/Application Support/Saya/saya.db`).
#[uniffi::export]
pub fn default_db_path() -> String {
    saya_core::paths::default_db_path()
        .to_string_lossy()
        .into_owned()
}

/// Default log file path (`~/Library/Logs/Saya/saya.log`).
#[uniffi::export]
pub fn default_log_path() -> String {
    saya_core::paths::default_log_path()
        .to_string_lossy()
        .into_owned()
}

/// Emit a log line from the Swift side into the shared tracing pipeline.
/// `level` is one of "error" | "warn" | "info" | "debug" | "trace"
/// (unknown levels fall through to info).
#[uniffi::export]
pub fn log_from_swift(level: String, message: String) {
    logging::init();
    match level.as_str() {
        "error" => tracing::error!(target: "saya_ui", "{message}"),
        "warn"  => tracing::warn!(target:  "saya_ui", "{message}"),
        "debug" => tracing::debug!(target: "saya_ui", "{message}"),
        "trace" => tracing::trace!(target: "saya_ui", "{message}"),
        _       => tracing::info!(target:  "saya_ui", "{message}"),
    }
}
