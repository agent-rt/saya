# Changelog

## v0.0.2 — devloop & UX polish

**New**

- **DevServer**: local JSON-RPC control plane on `127.0.0.1:7896` for automation and debugging
  (`panel.open/close/toggle`, `input.set/submit`, `launcher.snapshot`, `clipboard.snapshot`,
  `state`, `settings.open`, `event.subscribe` with streaming events).
- `saya dev <method>` CLI for one-shot RPC + `--follow` for live event streams.
- End-to-end test suite (`just e2e`, 19 tests) driving the running app through RPC.
- Unified logging to `~/Library/Logs/Saya/saya.log` with `SAYA_LOG` env filter; Swift events
  funnel into the same Rust tracing pipeline via `log_from_swift`. `saya logs [-f]` to tail.
- URL scheme: `open saya://launcher` / `saya://clipboard`.
- Rust → Swift realtime clipboard observer; new entries land in the panel without a refresh roundtrip.
- Empty-query launcher now sorts by MRU (most recent / frequent first), then alphabetical.

**Fixed**

- Chrome (and other apps) showing the wrong icon in launcher: SwiftUI was reusing AppRow views
  across different apps because ForEach keyed rows by ordinal index; the per-row `@State icon`
  leaked between apps. Now `LauncherResult` is `Identifiable` and rows are keyed by
  `app:<path>` / `calc:<expression>`.
- Icon prefetch cold path went from ~104s (NSWorkspace serialised across threads) to ~4s by
  reading `.icns` directly from the app bundle. Atomic writes (tmp + rename).
- TextField focus on hotkey re-open: panels are reused (`orderOut` keeps the SwiftUI tree
  mounted), and `.onAppear` / `.task` don't refire reliably. `PanelController.show` now bumps
  a `*FocusTrigger` counter that the views observe.
- Empty launcher after switching panels: same root cause — explicit `refreshLauncher` /
  `refreshRecent` on every `show`.
- Phantom hover stealing selection: typing a query re-renders the list and the row sliding under
  a stationary cursor fired `.onHover(true)`, overriding the keyboard default to row 0.
  Hover now only changes selection when `NSEvent.mouseLocation` actually moved.
- Two panels overlapping when invoked from tray: `show(kind)` now closes any other visible kind.
- Tray menu actions are now `show()` (not `toggle()`) so they always show on the first click,
  even mid-transition from `hidesOnDeactivate`.
- Slow startup (`几秒才出现tray`): reinstated `LSUIElement=true` now that we run our own
  `SettingsWindowController` instead of SwiftUI's Settings scene — Release startup back to ~1.75s.
- Toned-down selection highlight (no accent stripe; primary @ 6% opacity).
- Launcher `code` query: word-prefix bonus now correctly ranks `Codex` > `Visual Studio Code` >
  `Claude Code URL Handler` > sparse subseqs.
- `r2d2 database is locked` race on first launch: schema migration runs in a single bootstrap
  connection before the pool is built.

**Architecture changes**

- Launcher state hoisted into `AppState` (`launcherQuery`, `launcherApps`, `launcherCalc`,
  `launcherSelected`) so the DevServer can observe and mutate it — no more "is it visible? what
  did we render?" guesswork.
- `SettingsWindowController` replaces SwiftUI's `Settings` scene; brings up its own NSWindow
  and toggles `NSApp.activationPolicy` between `.accessory` and `.regular`.
- Migrated from `ObservableObject` to `@Observable` (Swift Observation framework) for
  finer-grained view invalidation.

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
