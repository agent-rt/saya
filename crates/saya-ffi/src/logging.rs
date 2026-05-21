//! Process-wide logging setup. Idempotent; safe to call repeatedly.
//!
//! Two sinks active by default:
//! - stderr (visible when launching the .app from a terminal)
//! - `~/Library/Logs/Saya/saya.log` (persisted, append-only; bundled into
//!   support reports)
//!
//! Override the filter via the `SAYA_LOG` env var (RUST_LOG syntax):
//!     SAYA_LOG=saya_core=debug,saya_ffi=trace
//!
//! The default filter keeps known-noisy third-party crates at WARN so user
//! logs stay readable without sacrificing visibility into our own code.

use std::fs::{File, OpenOptions};
use std::sync::{Mutex, Once};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

static LOGGING_INIT: Once = Once::new();

const DEFAULT_FILTER: &str = concat!(
    "saya_core=info,",
    "saya_ffi=info,",
    "saya_ui=info,",
    "tantivy=warn,",
    "candle_core=warn,",
    "candle_nn=warn,",
    "candle_transformers=warn,",
    "hf_hub=warn,",
    "ureq=warn,",
    "rustls=warn,",
    "reqwest=warn,",
    "warn",
);

pub fn init() {
    LOGGING_INIT.call_once(|| {
        let filter = EnvFilter::try_from_env("SAYA_LOG")
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

        let stderr_layer = fmt::layer()
            .with_target(true)
            .with_level(true)
            .with_ansi(true)
            .with_writer(std::io::stderr);

        let registry = tracing_subscriber::registry().with(filter).with(stderr_layer);

        // File sink: best-effort. Falls back to stderr-only if open fails.
        match open_log_file() {
            Some(file) => {
                let file_layer = fmt::layer()
                    .with_target(true)
                    .with_level(true)
                    .with_ansi(false)
                    .with_writer(Mutex::new(file));
                registry.with(file_layer).init();
            }
            None => {
                registry.init();
            }
        }

        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            log_path = %saya_core::paths::default_log_path().display(),
            "saya logging initialised"
        );
    });
}

fn open_log_file() -> Option<File> {
    let log_path = saya_core::paths::default_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()
}
