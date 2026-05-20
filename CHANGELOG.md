# Changelog

## v0.0.1 — first cut

Initial MVP. macOS 15+ on Apple Silicon.

**Features**

- App launcher with fuzzy matching, MRU bias, FSEvents incremental updates
- Clipboard history with hybrid search (Tantivy BM25 + jieba-rs tokenizer + sqlite-vec cosine via Reciprocal Rank Fusion)
- Local semantic search via Candle MiniLM-L6-v2 (opt-in, Metal-accelerated, idle-unloads)
- Spotlight-style floating panel: global hotkeys (`⌥ Space` / `⌥⇧ V`), `⌘1-9` quick select, mouse hover sync, ↵ to act, Esc to dismiss
- Inline calculator (`+ - * / ( ) %`)
- Multi-layer caching: NSImage RAM cache, PNG disk cache, apps index L2, MRU FFI cache, real-time clipboard observer

**Architecture**

- `saya-core` — pure-Rust library (DB, AI, clipboard monitor, launcher index, hybrid search)
- `saya-ffi` — UniFFI surface, opaque `Saya` object exposed to Swift
- `saya-cli` — terminal companion (`saya search`, `saya launch`, `saya watch`, …)
- `saya-macos` — SwiftUI shell wrapping the FFI; statically links the Rust core

**Limits**

- macOS 15 (Sequoia) + Apple Silicon only
- Ad-hoc signed (Gatekeeper "downloaded from internet" prompt on first launch)
- Embedding feature off by default; enable in Settings
