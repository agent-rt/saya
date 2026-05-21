import AppKit
import KeyboardShortcuts
import SwiftUI

@main
struct SayaApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra("Saya", systemImage: "magnifyingglass") {
            MenuContent()
                .environment(delegate.state)
                .environment(delegate.panels)
        }
        .menuBarExtraStyle(.menu)
    }
}

/// Owns long-lived state and wires global hotkeys on launch. Using
/// NSApplicationDelegate (instead of `MenuContent.onAppear`) means the
/// hotkeys are armed before the user ever clicks the menu bar icon, so the
/// first press can't outrun setup.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let state = AppState()
    let panels = PanelController()
    var devServer: DevServer?

    override init() {
        super.init()
        Self.fileTrace("AppDelegate.init")
        Log.info("AppDelegate.init")
        NotificationCenter.default.addObserver(
            forName: NSApplication.didFinishLaunchingNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Self.fileTrace("NotificationCenter: didFinishLaunching")
            Log.info("NotificationCenter saw NSApplication.didFinishLaunchingNotification")
            Task { @MainActor in
                self?.setupAfterLaunch()
            }
        }

        // Promote to `.regular` while a non-panel window is on screen (so
        // macOS shows the Saya app menu + Dock icon). Drop back to
        // `.accessory` once the last such window closes — keeps the
        // Spotlight-style "no Dock, no top menu" feel for the floating panel.
        NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeMainNotification,
            object: nil, queue: .main
        ) { note in
            Task { @MainActor in AppDelegate.windowBecameMain(note) }
        }
        NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification,
            object: nil, queue: .main
        ) { note in
            Task { @MainActor in AppDelegate.windowWillClose(note) }
        }
    }

    @MainActor
    private static func windowBecameMain(_ note: Notification) {
        guard let win = note.object as? NSWindow, !(win is SayaPanel) else { return }
        if NSApp.activationPolicy() != .regular {
            Log.info("window became main: \(win.identifier?.rawValue ?? "?"); → .regular")
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    @MainActor
    private static func windowWillClose(_ note: Notification) {
        guard let closing = note.object as? NSWindow, !(closing is SayaPanel) else { return }
        // Check if any other non-panel window is still alive after this one closes.
        let othersAlive = NSApp.windows.contains { w in
            w !== closing && !(w is SayaPanel) && w.isVisible
        }
        if !othersAlive && NSApp.activationPolicy() != .accessory {
            Log.info("last regular window closed; → .accessory")
            NSApp.setActivationPolicy(.accessory)
        }
    }

    static func fileTrace(_ msg: String) {
        let url = URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent("Library/Logs/Saya/swift-trace.log")
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                  withIntermediateDirectories: true)
        let line = "[\(Date().timeIntervalSince1970)] \(msg)\n"
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile()
            h.write(Data(line.utf8))
            try? h.close()
        } else {
            try? line.data(using: .utf8)?.write(to: url)
        }
    }

    private var didSetup = false
    private func setupAfterLaunch() {
        guard !didSetup else { return }
        didSetup = true
        Log.info("AppDelegate: running setup")
        panels.bind(state)
        KeyboardShortcuts.onKeyDown(for: .toggleLauncher) { [panels, state] in
            Log.info("hotkey: toggle launcher")
            state.eventSink?("hotkey.toggleLauncher", [:])
            // Fresh query for user-initiated invocations.
            state.resetLauncher()
            panels.toggle(.launcher)
        }
        KeyboardShortcuts.onKeyDown(for: .toggleClipboard) { [panels, state] in
            Log.info("hotkey: toggle clipboard")
            state.eventSink?("hotkey.toggleClipboard", [:])
            panels.toggle(.clipboard)
        }
        Task.detached { [state] in
            state.saya.setClipboardObserver(observer: state)
        }
        // CDP-like local JSON-RPC for dev / automation / smoke tests.
        let server = DevServer(state: state, panels: panels)
        server.start()
        devServer = server
        Log.info("app ready: hotkeys armed, panels wired, dev server up")
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        Self.fileTrace("applicationDidFinishLaunching")
        Log.info("applicationDidFinishLaunching fired")
        setupAfterLaunch()
    }

    /// Handles `saya://launcher` / `saya://clipboard` URL invocations.
    /// Useful for automation (devloop, Shortcuts.app) since synthetic
    /// Carbon hotkey events are filtered by modern macOS.
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            Log.info("open URL: \(url.absoluteString)")
            // Make sure setup ran in case the URL launches the app cold.
            setupAfterLaunch()
            switch url.host {
            case "launcher": panels.show(.launcher)
            case "clipboard": panels.show(.clipboard)
            default:
                Log.warn("unknown URL host: \(url.host ?? "nil")")
            }
        }
    }
}

private struct MenuContent: View {
    @Environment(AppState.self) private var state
    @Environment(PanelController.self) private var panels

    var body: some View {
        // Menu items are labeled "Show X" — they should always show.
        // Toggling can mis-trigger during hidesOnDeactivate's async hide
        // transition (panel.isVisible briefly reports true, toggle picks
        // the "hide" branch, user has to click twice).
        Button("Show Launcher")  { panels.show(.launcher) }
        Button("Show Clipboard") { panels.show(.clipboard) }
        Divider()
        Button("Settings…") {
            SettingsWindowController.shared.show(state: state)
        }
        .keyboardShortcut(",")
        Divider()
        Button("Quit Saya") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}
