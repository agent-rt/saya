//! Application launcher: scan, match, launch.
//!
//! - Recursively walk /Applications, /System/Applications, ~/Applications
//!   (treating `.app` bundles as leaves).
//! - Index lives behind an `RwLock` so the FSEvents watcher can apply
//!   incremental updates without rebuilding the whole tree.
//! - Display name = bundle filename (matches user-visible name for ~all
//!   common apps; localized Info.plist is V1 territory).
//! - Match scoring: prefix / word-boundary / consecutive-hit bonuses, plus an
//!   MRU bias when a launch-history snapshot is provided.
//! - Icon extraction returns PNG bytes via objc2 → NSWorkspace.
//! - Launch shells out to `/usr/bin/open`.

mod matcher;
mod scanner;
mod watcher;

#[cfg(target_os = "macos")]
mod macos;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::database::MruInfo;

pub use scanner::default_roots;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedApp {
    pub app: AppEntry,
    pub score: i32,
}

pub struct LauncherIndex {
    apps: Arc<RwLock<Vec<AppEntry>>>,
    // Held to keep the FSEvents thread alive for the index's lifetime.
    _watcher: Option<notify::RecommendedWatcher>,
}

impl LauncherIndex {
    /// Build against the default macOS app roots. Uses the on-disk L2 cache
    /// (`~/Library/Caches/Saya/apps.json`) for instant startup; reconciles
    /// in the background against a fresh filesystem scan.
    pub fn build() -> crate::Result<Self> {
        Self::build_inner(&default_roots(), true)
    }

    /// Build against arbitrary roots without touching the L2 cache. Used by
    /// tests and the FSEvents integration example.
    pub fn build_from(roots: &[PathBuf]) -> crate::Result<Self> {
        Self::build_inner(roots, false)
    }

    fn build_inner(roots: &[PathBuf], use_l2: bool) -> crate::Result<Self> {
        let started = std::time::Instant::now();
        let (initial, from_l2) = if use_l2 {
            match scanner::load_cached_apps() {
                Some(cached) if !cached.is_empty() => (cached, true),
                _ => (scanner::scan(roots), false),
            }
        } else {
            (scanner::scan(roots), false)
        };
        if use_l2 && !from_l2 {
            scanner::save_cached_apps(&initial);
        }
        tracing::info!(
            count = initial.len(),
            source = if from_l2 { "L2 cache" } else { "filesystem" },
            elapsed = ?started.elapsed(),
            "launcher index built"
        );

        let apps = Arc::new(RwLock::new(initial));

        let watcher = match watcher::spawn(roots, apps.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(error = %e, "FSEvents watcher disabled; index will be static");
                None
            }
        };

        if from_l2 {
            // L2 may lag the filesystem (apps installed/removed since last
            // run). Reconcile in the background so the user gets immediate
            // startup and eventual consistency.
            let roots = roots.to_vec();
            let apps_handle = apps.clone();
            std::thread::Builder::new()
                .name("saya-apps-reconcile".into())
                .spawn(move || {
                    let fresh = scanner::scan(&roots);
                    let current = apps_handle.read().expect("apps lock").clone();
                    if current != fresh {
                        *apps_handle.write().expect("apps lock") = fresh.clone();
                        scanner::save_cached_apps(&fresh);
                        tracing::info!(
                            was = current.len(),
                            now = fresh.len(),
                            "apps index reconciled with disk"
                        );
                    }
                })
                .ok();
        }

        Ok(Self { apps, _watcher: watcher })
    }

    /// Snapshot of the current index. Cheap because AppEntry is small.
    pub fn apps(&self) -> Vec<AppEntry> {
        self.apps.read().expect("apps lock").clone()
    }

    /// Warm the on-disk icon cache for every known app. Runs on a dedicated
    /// background thread; safe to call repeatedly (cache lookups short-circuit
    /// already-warm entries).
    ///
    /// Extraction is parallelised across a small rayon pool (4 workers).
    /// NSWorkspace's `iconForFile` and NSBitmapImageRep are documented as
    /// thread-safe for read; limiting concurrency to 4 keeps AppKit happy
    /// while still hitting an ~4× speedup on cold start (~5s → ~1.3s for
    /// 122 apps).
    #[cfg(target_os = "macos")]
    pub fn prefetch_icons(&self) {
        let apps = self.apps.read().expect("apps lock").clone();
        std::thread::Builder::new()
            .name("saya-icon-prefetch".into())
            .spawn(move || {
                use rayon::prelude::*;
                let started = std::time::Instant::now();
                let pool = match rayon::ThreadPoolBuilder::new()
                    .num_threads(4)
                    .thread_name(|i| format!("saya-icon-{i}"))
                    .build()
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error = %e, "rayon pool build failed; sequential fallback");
                        let n = apps
                            .iter()
                            .filter(|a| macos::icon_png(&a.path).is_ok())
                            .count();
                        tracing::info!(
                            warmed = n,
                            total = apps.len(),
                            elapsed = ?started.elapsed(),
                            "icon prefetch complete (sequential)"
                        );
                        return;
                    }
                };
                let warmed: usize = pool.install(|| {
                    apps.par_iter()
                        .filter(|a| macos::icon_png(&a.path).is_ok())
                        .count()
                });
                tracing::info!(
                    warmed,
                    total = apps.len(),
                    elapsed = ?started.elapsed(),
                    "icon prefetch complete"
                );
            })
            .ok();
    }

    #[cfg(not(target_os = "macos"))]
    pub fn prefetch_icons(&self) {}

    pub fn match_query(
        &self,
        query: &str,
        limit: usize,
        mru: &HashMap<String, MruInfo>,
    ) -> Vec<MatchedApp> {
        let apps = self.apps.read().expect("apps lock");
        if query.is_empty() {
            // No query: rank by MRU (most recently / frequently used on top).
            // Never-launched apps tie at score 0 and fall back to alphabetical
            // order — which is also the order the underlying `apps` vec is
            // already kept in, so the secondary sort is essentially free.
            let mut scored: Vec<MatchedApp> = apps
                .iter()
                .map(|app| {
                    let key = app.path.to_string_lossy();
                    let score = mru
                        .get(key.as_ref())
                        .copied()
                        .map(matcher::mru_bonus)
                        .unwrap_or(0);
                    MatchedApp { app: app.clone(), score }
                })
                .collect();
            scored.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| a.app.name.to_lowercase().cmp(&b.app.name.to_lowercase()))
            });
            scored.truncate(limit);
            return scored;
        }
        let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
        let mut scored: Vec<MatchedApp> = apps
            .iter()
            .filter_map(|app| {
                let path_key = app.path.to_string_lossy();
                let m = mru.get(path_key.as_ref()).copied();
                matcher::score(&q, &app.name, m).map(|score| MatchedApp {
                    app: app.clone(),
                    score,
                })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.app.name.len().cmp(&b.app.name.len()))
                .then_with(|| a.app.name.cmp(&b.app.name))
        });
        scored.truncate(limit);
        scored
    }
}

#[cfg(target_os = "macos")]
pub use macos::{icon_png, launch};

#[cfg(not(target_os = "macos"))]
pub fn launch(_path: &std::path::Path) -> crate::Result<()> {
    Err(crate::Error::Other("launcher is macOS-only in MVP".into()))
}

#[cfg(not(target_os = "macos"))]
pub fn icon_png(_path: &std::path::Path) -> crate::Result<Vec<u8>> {
    Err(crate::Error::Other("launcher is macOS-only in MVP".into()))
}
