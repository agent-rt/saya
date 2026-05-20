//! NSPasteboard monitor — polls `changeCount` every 300ms via objc2.
//! Captures `public.utf8-plain-text` only in MVP.

#[cfg(target_os = "macos")]
mod macos_impl;

#[cfg(target_os = "macos")]
pub use macos_impl::{ClipboardMonitor, OnInsert, POLL_INTERVAL, MAX_BYTES};
