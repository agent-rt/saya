import AppKit
import SwiftUI

/// Floating Spotlight-style panel that hosts a SwiftUI view. Auto-dismisses
/// when it loses key (`hidesOnDeactivate`) and on Esc.
final class SayaPanel: NSPanel {
    init(contentSize: NSSize) {
        super.init(
            contentRect: NSRect(origin: .zero, size: contentSize),
            styleMask: [.borderless, .fullSizeContentView, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        level = .floating
        // Translucent — SwiftUI content provides the .regularMaterial background.
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        isMovableByWindowBackground = true
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        hidesOnDeactivate = true
        isReleasedWhenClosed = false
        animationBehavior = .utilityWindow
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func cancelOperation(_ sender: Any?) {
        // Esc → hide
        orderOut(nil)
    }
}

@Observable
@MainActor
final class PanelController {
    enum Kind: String, CaseIterable {
        case launcher, clipboard
    }

    private var panels: [Kind: SayaPanel] = [:]
    private weak var appState: AppState?

    func bind(_ state: AppState) {
        self.appState = state
    }

    func toggle(_ kind: Kind) {
        if let panel = panels[kind], panel.isVisible {
            panel.orderOut(nil)
        } else {
            show(kind)
        }
    }

    func show(_ kind: Kind) {
        let panel: SayaPanel
        if let existing = panels[kind] {
            panel = existing
        } else if let new = makePanel(kind) {
            panel = new
            panels[kind] = new
        } else {
            return
        }
        position(panel)
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    private func makePanel(_ kind: Kind) -> SayaPanel? {
        guard let appState else {
            NSLog("PanelController: toggle called before bind() — ignoring")
            return nil
        }
        let panel = SayaPanel(contentSize: NSSize(width: 680, height: 500))
        let dismiss: () -> Void = { [weak panel] in panel?.orderOut(nil) }
        let host: NSView
        switch kind {
        case .launcher:
            host = NSHostingView(
                rootView: LauncherView(onDismiss: dismiss).environment(appState)
            )
        case .clipboard:
            host = NSHostingView(
                rootView: ClipboardView(onDismiss: dismiss).environment(appState)
            )
        }
        // Fill the panel and keep doing so when it resizes.
        host.autoresizingMask = [.width, .height]
        panel.contentView = host
        return panel
    }

    /// Center horizontally, vertically biased toward the top third (Spotlight-like).
    private func position(_ panel: NSPanel) {
        guard let screen = NSScreen.main else { return }
        let panelSize = panel.frame.size
        let f = screen.visibleFrame
        let x = f.midX - panelSize.width / 2
        let y = f.minY + (f.height * 2 / 3) - panelSize.height / 2
        panel.setFrameOrigin(NSPoint(x: x, y: y))
    }
}
