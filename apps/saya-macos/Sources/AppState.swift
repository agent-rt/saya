import AppKit
import Foundation
import Observation

/// Owns the bridge to the Rust core and the per-session UI caches.
///
/// Migration notes:
/// - `@Observable` (macOS 14+) replaces `ObservableObject` so SwiftUI only
///   invalidates the specific views that read a given property.
/// - Caches are marked `@ObservationIgnored` so mutating them does not
///   trigger any view updates.
@Observable
@MainActor
final class AppState {
    let saya: Saya

    var status: StatusDto?
    var lastError: String?

    /// Recent clipboard entries, kept hot in memory so the panel pops up with
    /// content already drawn. Initial value `[]` means "not yet loaded";
    /// `recent.isEmpty` after the initial load truly means empty history.
    var recent: [ClipboardEntryDto] = []

    // MARK: - Launcher state (hoisted so DevServer can drive it)

    var launcherQuery: String = ""
    var launcherCalc: CalcResult?
    var launcherApps: [MatchedAppDto] = []
    var launcherSelected: Int = 0

    /// Bumped by `PanelController.show` so views can re-acquire focus on
    /// every panel display, including reused-panel cases where SwiftUI's
    /// `.onAppear` doesn't refire.
    var launcherFocusTrigger: Int = 0
    var clipboardFocusTrigger: Int = 0

    /// View-visible item list: calc row prepended (if any) + app rows.
    var launcherItems: [LauncherResult] {
        var out: [LauncherResult] = []
        if let c = launcherCalc { out.append(.calc(c)) }
        out.append(contentsOf: launcherApps.map { .app($0) })
        return out
    }

    // MARK: - Caches (untracked)

    @ObservationIgnored
    private var iconCache: [String: NSImage] = [:]

    @ObservationIgnored
    private var iconLoads: [String: Task<NSImage?, Never>] = [:]

    /// Event broadcast hook — DevServer wires this to feed subscribers.
    @ObservationIgnored
    var eventSink: ((String, [String: Any]) -> Void)?

    // MARK: - Init

    init() {
        // Use stderr directly here — Log.info goes through FFI which is not
        // available until Saya() returns, so we'd lose this trace.
        FileHandle.standardError.write(Data("[swift] AppState.init: before Saya()\n".utf8))
        let path = defaultDbPath()
        do {
            self.saya = try Saya(dbPath: path)
        } catch {
            fatalError("Saya init failed at \(path): \(error)")
        }
        refreshStatus()
        // Pre-extract every app icon on disk (Rust prefetch_icons does the
        // NSWorkspace + PNG-encode work once and lays them down in the disk
        // cache so the entire session is read-only after that).
        Task.detached { [saya] in
            try? saya.prefetchIcons()
        }
        // Pre-decode the first 50 icons into NSImage. That's the full result
        // count we ask for on empty query, so the launcher panel can render
        // its initial state with zero IO and zero decode.
        Task { [weak self] in
            await self?.prewarmIcons(count: 50)
        }
        // Pre-fetch the latest clipboard entries so opening the panel needs
        // no SQL roundtrip.
        Task { [weak self] in
            await self?.refreshRecent()
        }
    }

    // MARK: - Status

    func refreshStatus() {
        Task.detached { [saya] in
            let s = try? saya.status()
            await MainActor.run { self.status = s }
        }
    }

    // MARK: - Search

    func searchClipboard(_ query: String, semantic: Bool, limit: UInt32 = 50) async -> [SearchHitDto] {
        await run { saya in
            try saya.search(query: query, limit: limit, semantic: semantic)
        } ?? []
    }

    func recentClipboard(_ limit: UInt32 = 50) async -> [ClipboardEntryDto] {
        await run { saya in
            try saya.recentClipboard(limit: limit)
        } ?? []
    }

    /// Refresh the cached `recent` list in place. Safe to call any time; the
    /// UI sees the new value once it lands. Used both for initial population
    /// and on panel open to catch entries captured while the panel was hidden.
    func refreshRecent(_ limit: UInt32 = 50) async {
        let r = await recentClipboard(limit)
        recent = r
    }

    // MARK: - Launcher

    func matchApps(_ query: String, limit: UInt32 = 8) async -> [MatchedAppDto] {
        await run { saya in
            try saya.matchApps(query: query, limit: limit)
        } ?? []
    }

    func allApps() async -> [AppEntryDto] {
        await run { saya in try saya.apps() } ?? []
    }

    func launch(_ path: String) {
        Task.detached { [saya] in
            do { try saya.launchApp(path: path) }
            catch { await self.report(error) }
        }
    }

    func iconPng(_ path: String) async -> Data? {
        await run { saya in try saya.iconPng(path: path) }
    }

    /// Returns a cached `NSImage`, decoding once on the first request.
    /// Concurrent requests for the same path coalesce so we don't fan out
    /// duplicate PNG decodes.
    func iconImage(_ path: String) async -> NSImage? {
        if let cached = iconCache[path] { return cached }
        if let inflight = iconLoads[path] { return await inflight.value }
        let task: Task<NSImage?, Never> = Task.detached(priority: .userInitiated) { [saya] in
            guard let data = try? saya.iconPng(path: path) else { return nil }
            return NSImage(data: data)
        }
        iconLoads[path] = task
        let image = await task.value
        iconLoads.removeValue(forKey: path)
        if let image { iconCache[path] = image }
        return image
    }

    /// Pre-decode top-N app icons into the in-memory cache. Runs in the
    /// background; safe to call without awaiting. Idempotent (cache hits
    /// short-circuit immediately).
    func prewarmIcons(count: Int) async {
        let apps = await allApps()
        for app in apps.prefix(count) {
            _ = await iconImage(app.path)
        }
    }

    // MARK: - Clipboard monitor

    func startClipboardMonitor(embed: Bool) {
        Task.detached { [saya] in
            do { try saya.startClipboardMonitor(embed: embed) }
            catch { await self.report(error) }
            await MainActor.run { self.refreshStatus() }
        }
    }

    func stopClipboardMonitor() {
        Task.detached { [saya] in
            saya.stopClipboardMonitor()
            await MainActor.run { self.refreshStatus() }
        }
    }

    func reindex(limit: UInt32 = 1000, batch: UInt32 = 16) async -> UInt32 {
        await run { saya in try saya.reindex(limit: limit, batch: batch) } ?? 0
    }

    func unloadEmbedder() {
        Task.detached { [saya] in
            saya.unloadEmbedder()
            await MainActor.run { self.refreshStatus() }
        }
    }

    // MARK: - Clipboard write

    func copyToPasteboard(_ text: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(text, forType: .string)
    }

    // MARK: - Helpers

    private func run<T: Sendable>(_ body: @escaping @Sendable (Saya) throws -> T) async -> T? {
        do {
            return try await Task.detached(priority: .userInitiated) { [saya] in
                try body(saya)
            }.value
        } catch {
            await report(error)
            return nil
        }
    }

    private func report(_ error: Error) async {
        let msg = error.localizedDescription
        await MainActor.run { self.lastError = msg }
    }

    func clearError() { lastError = nil }

    // MARK: - Launcher pipeline

    /// Re-run the launcher pipeline for the current `launcherQuery`. Idempotent;
    /// callers don't need to debounce — concurrent calls are coalesced by the
    /// stamp check so only the latest query's results land in state.
    func refreshLauncher() async {
        let q = launcherQuery
        let limit: UInt32 = q.isEmpty ? 50 : 12
        let calc = Calculator.evaluate(q).map {
            CalcResult(expression: q.trimmingCharacters(in: .whitespaces),
                       value: Calculator.format($0))
        }
        let apps = await matchApps(q, limit: limit)
        guard launcherQuery == q else { return }
        launcherCalc = calc
        launcherApps = apps
        launcherSelected = 0
    }

    /// Execute the currently-selected launcher item (calc → copy, app → launch).
    /// Returns a short description of what ran for RPC consumers.
    @discardableResult
    func executeLauncherSelection() -> String {
        let items = launcherItems
        guard items.indices.contains(launcherSelected) else { return "none" }
        let result: String
        let payload: [String: Any]
        switch items[launcherSelected] {
        case .calc(let c):
            copyToPasteboard(c.value)
            result = "calc:\(c.value)"
            payload = ["kind": "calc", "expression": c.expression, "value": c.value]
        case .app(let a):
            launch(a.path)
            result = "app:\(a.name)"
            payload = ["kind": "app", "name": a.name, "path": a.path]
        }
        eventSink?("launcher.executed", payload)
        return result
    }

    /// Clear launcher state — called when the user invokes the panel fresh
    /// via the global hotkey (so they don't see the previous query).
    func resetLauncher() {
        launcherQuery = ""
        launcherCalc = nil
        launcherApps = []
        launcherSelected = 0
    }
}

// MARK: - ClipboardObserver bridge

extension AppState: ClipboardObserver {
    /// Called by Rust on the monitor polling thread when a new entry lands.
    /// We hop to @MainActor before touching the observable `recent` array.
    nonisolated func onEntryCaptured(entry: ClipboardEntryDto) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.recent.insert(entry, at: 0)
            if self.recent.count > 50 {
                self.recent.removeLast(self.recent.count - 50)
            }
            self.eventSink?("clipboard.captured", [
                "id": Int(entry.id),
                "content": entry.content,
                "ts": Int(entry.createdAtUnixMs),
            ])
        }
    }
}
