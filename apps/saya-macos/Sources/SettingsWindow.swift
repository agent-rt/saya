import AppKit
import SwiftUI

/// Manual AppKit-backed Settings window.
///
/// Owns its own activation-policy lifecycle so the top menubar reliably
/// switches to Saya's menus while Settings is on screen — and switches back
/// to `.accessory` (no Dock, no menubar) when the user closes it.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    static let shared = SettingsWindowController()
    private var window: NSWindow?

    func show(state: AppState) {
        if window == nil {
            let win = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 560, height: 520),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                backing: .buffered,
                defer: false
            )
            win.title = "Saya Settings"
            win.center()
            win.isReleasedWhenClosed = false
            win.delegate = self
            win.contentView = NSHostingView(rootView: SettingsView().environment(state))
            window = win
            Log.info("SettingsWindow: created")
        }
        // Promote BEFORE making the window key so AppKit has a chance to
        // attach the app menu by the time the window is on screen.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
        Log.info("SettingsWindow: shown, policy=.regular")
    }

    nonisolated func windowWillClose(_ notification: Notification) {
        Task { @MainActor in
            Log.info("SettingsWindow: closing → .accessory")
            NSApp.setActivationPolicy(.accessory)
        }
    }
}
