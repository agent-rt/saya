use std::path::PathBuf;

/// `~/Library/Application Support/Saya/`
pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/Saya")
}

pub fn default_db_path() -> PathBuf {
    data_dir().join("saya.db")
}

/// `~/Library/Caches/Saya/`. Cached artifacts that can be regenerated on
/// demand (icons, model artifacts, etc.).
pub fn cache_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Caches/Saya")
}

pub fn icon_cache_dir() -> PathBuf {
    cache_dir().join("icons")
}

/// `~/Library/Logs/Saya/`. Append-only operational logs.
pub fn log_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Logs/Saya")
}

pub fn default_log_path() -> PathBuf {
    log_dir().join("saya.log")
}
