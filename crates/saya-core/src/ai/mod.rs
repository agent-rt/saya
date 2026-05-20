//! Candle-based local embedding (all-MiniLM-L6-v2, 384 dim).
//!
//! Gated behind the `embedding` cargo feature. Lazy-loaded by `EmbedderHandle`,
//! unloaded after idle to release Metal context. Model weights are fetched on
//! first use via `hf-hub`, which caches under `~/.cache/huggingface/hub/`.

pub const EMBEDDING_DIM: usize = 384;

#[cfg(feature = "embedding")]
mod embedder;

#[cfg(feature = "embedding")]
pub use embedder::{Embedder, EmbedderHandle, IDLE_TIMEOUT, MODEL_REPO};
