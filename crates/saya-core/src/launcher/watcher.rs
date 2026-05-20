//! FSEvents-driven incremental updates for the launcher index.
//!
//! Watches the three root dirs recursively. Each event is collapsed to its
//! enclosing `.app` bundle path; then we *re-derive truth from the
//! filesystem* via `path.exists()` rather than trusting the event kind. This
//! makes us robust against the many spurious child events FSEvents emits
//! inside `.app` bundles (Spotlight indexing, signature refresh, etc.).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use super::AppEntry;

pub fn spawn(
    roots: &[PathBuf],
    apps: Arc<RwLock<Vec<AppEntry>>>,
) -> notify::Result<RecommendedWatcher> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    for root in roots {
        if root.exists() {
            // Recursive: we'll get many internal events, but extract_app_path
            // collapses each to the .app boundary and we dedup via vec membership.
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                tracing::warn!(root = %root.display(), error = %e, "fsevents watch failed");
            }
        }
    }
    std::thread::Builder::new()
        .name("saya-launcher-watcher".into())
        .spawn(move || {
            for res in rx {
                match res {
                    Ok(event) => handle(event, &apps),
                    Err(e) => tracing::warn!(error = %e, "fsevents error"),
                }
            }
        })
        .ok();
    Ok(watcher)
}

fn handle(event: Event, apps: &RwLock<Vec<AppEntry>>) {
    for path in &event.paths {
        let Some(app_path) = extract_app_path(path) else { continue };
        if app_path.exists() {
            insert(&app_path, apps);
        } else {
            remove(&app_path, apps);
        }
    }
}

fn extract_app_path(p: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for c in p.components() {
        result.push(c);
        if result.extension().and_then(|s| s.to_str()) == Some("app") {
            return Some(result);
        }
    }
    None
}

fn insert(path: &Path, apps: &RwLock<Vec<AppEntry>>) {
    let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
        return;
    };
    let entry = AppEntry {
        name,
        path: path.to_path_buf(),
    };
    let mut guard = apps.write().expect("apps lock");
    if guard.iter().any(|a| a.path == entry.path) {
        return;
    }
    let key = entry.name.to_lowercase();
    let pos = guard
        .binary_search_by(|a| a.name.to_lowercase().cmp(&key))
        .unwrap_or_else(|p| p);
    tracing::debug!(path = %entry.path.display(), "launcher index: app added");
    guard.insert(pos, entry);
}

fn remove(path: &Path, apps: &RwLock<Vec<AppEntry>>) {
    let mut guard = apps.write().expect("apps lock");
    if let Some(idx) = guard.iter().position(|a| a.path == *path) {
        guard.remove(idx);
        tracing::debug!(path = %path.display(), "launcher index: app removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_app_path_handles_nested_paths() {
        assert_eq!(
            extract_app_path(Path::new("/Applications/Foo.app/Contents/MacOS/Foo")),
            Some(PathBuf::from("/Applications/Foo.app"))
        );
        assert_eq!(
            extract_app_path(Path::new("/Applications/Foo.app")),
            Some(PathBuf::from("/Applications/Foo.app"))
        );
        assert_eq!(extract_app_path(Path::new("/Applications/README.txt")), None);
    }

    #[test]
    fn insert_dedups_and_sorts() {
        let apps = Arc::new(RwLock::new(vec![
            AppEntry { name: "Bar".into(), path: PathBuf::from("/A/Bar.app") },
            AppEntry { name: "Foo".into(), path: PathBuf::from("/A/Foo.app") },
        ]));
        insert(Path::new("/A/Baz.app"), &apps);
        insert(Path::new("/A/Baz.app"), &apps); // duplicate, should be no-op
        let g = apps.read().unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].name, "Bar");
        assert_eq!(g[1].name, "Baz");
        assert_eq!(g[2].name, "Foo");
    }

    #[test]
    fn remove_drops_entry() {
        let apps = Arc::new(RwLock::new(vec![
            AppEntry { name: "Foo".into(), path: PathBuf::from("/A/Foo.app") },
        ]));
        remove(Path::new("/A/Foo.app"), &apps);
        assert!(apps.read().unwrap().is_empty());
    }
}
