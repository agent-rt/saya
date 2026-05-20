use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::AppEntry;

/// On-disk format version. Bump when AppEntry shape changes incompatibly.
const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "apps.json";

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    apps: Vec<AppEntry>,
}

pub fn default_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

pub fn scan(roots: &[PathBuf]) -> Vec<AppEntry> {
    let mut apps: Vec<AppEntry> = roots
        .par_iter()
        .flat_map_iter(|root| {
            let mut out = Vec::new();
            walk(root, &mut out);
            out
        })
        .collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.path == b.path);
    apps
}

/// Read the persisted apps cache if present and the version matches.
/// Returns `None` on any error or version mismatch (caller falls back to
/// a fresh scan).
pub fn load_cached_apps() -> Option<Vec<AppEntry>> {
    let path = cache_path();
    let bytes = std::fs::read(&path).ok()?;
    let cache: CacheFile = serde_json::from_slice(&bytes).ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    Some(cache.apps)
}

/// Persist the current apps snapshot atomically (write to tmp, rename).
pub fn save_cached_apps(apps: &[AppEntry]) {
    let cache = CacheFile {
        version: CACHE_VERSION,
        apps: apps.to_vec(),
    };
    let Ok(bytes) = serde_json::to_vec(&cache) else { return };
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn cache_path() -> PathBuf {
    crate::paths::cache_dir().join(CACHE_FILE)
}

fn walk(dir: &Path, out: &mut Vec<AppEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) == Some("app") {
            if let Some(name) = display_name(&path) {
                out.push(AppEntry { name, path });
            }
        } else if file_type.is_dir() {
            let basename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if matches!(basename, ".Trash" | "Library" | "Caches") {
                continue;
            }
            walk(&path, out);
        }
    }
}

fn display_name(app_path: &Path) -> Option<String> {
    app_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let original = vec![
            AppEntry { name: "Alpha".into(), path: PathBuf::from("/A/Alpha.app") },
            AppEntry { name: "Bravo".into(), path: PathBuf::from("/A/Bravo.app") },
        ];
        // Use serde directly so we don't depend on the well-known cache dir.
        let cache = CacheFile { version: CACHE_VERSION, apps: original.clone() };
        let bytes = serde_json::to_vec(&cache).unwrap();
        let parsed: CacheFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.version, CACHE_VERSION);
        assert_eq!(parsed.apps, original);
    }
}
