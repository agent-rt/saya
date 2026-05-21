import AppKit
import SwiftUI

/// Floating Spotlight-style panel that hosts a SwiftUI view. Auto-dismisses
/// when it loses key (`hidesOnDeactivate`) and on Esc.
final class SayaPanel: NSPanel {
    init(contentSize: NSSize) {
        super.init(
            contentRect: NSRect(origin: .zero, size: contentSize),
            // Removed `.nonactivatingPanel`: it suppressed app activation
            // when the panel became key, which prevented system-injected
            // keystrokes (and sometimes real ones) from reaching the
            // TextField. We now explicitly activate via show().
            styleMask: [.borderless, .fullSizeContentView],
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

    /// Event broadcast hook — DevServer wires this so subscribers can observe
    /// panel lifecycle. Nil by default (production builds).
    var eventSink: ((String, [String: Any]) -> Void)?

    func bind(_ state: AppState) {
        self.appState = state
    }

    func toggle(_ kind: Kind) {
        // If THIS kind is visible, hide it. Otherwise show it (which also
        // hides any other kind that's visible — see `show`).
        if let panel = panels[kind], panel.isVisible {
            Log.info("panel \(kind.rawValue): hide (toggle)")
            panel.orderOut(nil)
            eventSink?("panel.\(kind.rawValue).hidden", [:])
            return
        }
        show(kind)
    }

    /// Programmatically dismiss the panel (used by the DevServer's
    /// `panel.close` RPC and by `input.submit`).
    func close(_ kind: Kind) {
        if let panel = panels[kind], panel.isVisible {
            Log.info("panel \(kind.rawValue): close")
            panel.orderOut(nil)
            eventSink?("panel.\(kind.rawValue).hidden", [:])
        }
    }

    func show(_ kind: Kind) {
        Log.info("panel \(kind.rawValue): show")
        // Only one panel visible at a time — opening one closes the others
        // so the two never stack on screen.
        for other in Kind.allCases where other != kind {
            if let p = panels[other], p.isVisible {
                p.orderOut(nil)
                eventSink?("panel.\(other.rawValue).hidden", [:])
            }
        }
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
        NSRunningApplication.current.activate(options: [.activateAllWindows])
        panel.makeKeyAndOrderFront(nil)
        Log.info("panel \(kind.rawValue): isKey=\(panel.isKeyWindow) frontmostApp=\(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?")")
        eventSink?("panel.\(kind.rawValue).shown", [:])

        // Panels are reused across show/hide cycles (orderOut keeps the
        // NSHostingView mounted). SwiftUI `.task(id:)` and `.onAppear`
        // don't reliably refire when the underlying NSPanel goes
        // hidden→visible, so we explicitly refresh state and bump a focus
        // trigger here.
        if let state = appState {
            switch kind {
            case .launcher:  state.launcherFocusTrigger  &+= 1
            case .clipboard: state.clipboardFocusTrigger &+= 1
            }
            Task { @MainActor in
                switch kind {
                case .launcher:  await state.refreshLauncher()
                case .clipboard: await state.refreshRecent()
                }
            }
        }
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
