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

        Settings {
            SettingsView()
                .environment(delegate.state)
                .frame(minWidth: 480, minHeight: 520)
        }
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

    func applicationDidFinishLaunching(_ notification: Notification) {
        panels.bind(state)
        KeyboardShortcuts.onKeyDown(for: .toggleLauncher) { [panels] in
            panels.toggle(.launcher)
        }
        KeyboardShortcuts.onKeyDown(for: .toggleClipboard) { [panels] in
            panels.toggle(.clipboard)
        }
        // Register AppState as a real-time clipboard observer so new entries
        // appear in the panel without waiting for a manual refresh.
        Task.detached { [state] in
            state.saya.setClipboardObserver(observer: state)
        }
    }
}

private struct MenuContent: View {
    @Environment(AppState.self) private var state
    @Environment(PanelController.self) private var panels

    var body: some View {
        Button("Show Launcher") { panels.toggle(.launcher) }
        Button("Show Clipboard") { panels.toggle(.clipboard) }
        Divider()
        SettingsLink {
            Text("Settings…")
        }
        Divider()
        Button("Quit Saya") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}
