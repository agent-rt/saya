import AppKit
import SwiftUI

struct LauncherView: View {
    var onDismiss: () -> Void = {}

    @Environment(AppState.self) private var stateEnv
    @State private var selectionFromKeyboard: Bool = true
    @State private var lastHoverLocation: NSPoint?
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var state = stateEnv
        PanelChrome {
            VStack(spacing: 0) {
                PanelSearchField(
                    placeholder: "Search applications or calculate…",
                    text: $state.launcherQuery,
                    focused: $focused,
                    onSubmit: execute,
                    onUp: { moveSelection(-1) },
                    onDown: { moveSelection(1) },
                    onCmdDigit: { n in
                        let idx = n - 1
                        guard idx < state.launcherItems.count else { return false }
                        selectionFromKeyboard = true
                        state.launcherSelected = idx
                        execute()
                        return true
                    }
                )

                Divider().opacity(0.4)

                content

                Divider().opacity(0.4)

                PanelFooter(hints: footerHints)
            }
        }
        .task(id: state.launcherQuery) {
            await state.refreshLauncher()
        }
        .onAppear { focused = true }
        // Panel may be reused across show/hide; `.onAppear` doesn't refire
        // in that case, so the search field would stay defocused on second
        // open. PanelController bumps this trigger on every show.
        .onChange(of: state.launcherFocusTrigger) { _, _ in
            focused = true
        }
    }

    private var footerHints: [PanelFooter.Hint] {
        let primary: PanelFooter.Hint
        let items = stateEnv.launcherItems
        if items.indices.contains(stateEnv.launcherSelected),
           case .calc = items[stateEnv.launcherSelected] {
            primary = .init(key: "↵", label: "Copy result")
        } else {
            primary = .init(key: "↵", label: "Open")
        }
        return [primary, .init(key: "⎋", label: "Dismiss")]
    }

    @ViewBuilder
    private var content: some View {
        let items = stateEnv.launcherItems
        if items.isEmpty {
            if !stateEnv.launcherQuery.isEmpty {
                VStack {
                    Spacer()
                    Text("No matches").font(.callout).foregroundStyle(.secondary)
                    Spacer()
                }
            } else {
                Spacer()
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                            row(for: item, at: idx)
                                .contentShape(Rectangle())
                                .onHover { hovering in
                                    guard hovering else { return }
                                    let now = NSEvent.mouseLocation
                                    // Ignore "phantom hovers" — new rows
                                    // sliding under a stationary cursor on
                                    // re-render shouldn't steal selection
                                    // away from the keyboard default
                                    // (otherwise Enter opens whichever app
                                    // happens to be under the cursor).
                                    if let last = lastHoverLocation, last == now {
                                        return
                                    }
                                    lastHoverLocation = now
                                    selectionFromKeyboard = false
                                    stateEnv.launcherSelected = idx
                                }
                                .onTapGesture {
                                    selectionFromKeyboard = false
                                    stateEnv.launcherSelected = idx
                                    execute()
                                }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .onChange(of: stateEnv.launcherSelected) { _, new in
                    guard selectionFromKeyboard else { return }
                    guard items.indices.contains(new) else { return }
                    withAnimation(.easeOut(duration: 0.08)) {
                        proxy.scrollTo(items[new].id, anchor: .center)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func row(for item: LauncherResult, at idx: Int) -> some View {
        let shortcut = idx < 9 ? idx + 1 : nil
        let selected = idx == stateEnv.launcherSelected
        switch item {
        case .calc(let c):
            CalcRow(result: c, isSelected: selected, shortcut: shortcut)
        case .app(let app):
            AppRow(app: app, isSelected: selected, shortcut: shortcut)
        }
    }

    private func moveSelection(_ delta: Int) {
        let items = stateEnv.launcherItems
        guard !items.isEmpty else { return }
        selectionFromKeyboard = true
        stateEnv.launcherSelected = (stateEnv.launcherSelected + delta).clamped(to: 0...(items.count - 1))
    }

    private func execute() {
        stateEnv.executeLauncherSelection()
        onDismiss()
    }
}

// MARK: - Rows

private struct CalcRow: View {
    let result: CalcResult
    let isSelected: Bool
    let shortcut: Int?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "equal.circle.fill")
                .font(.system(size: 26))
                .foregroundStyle(Color.accentColor)
                .frame(width: 32, height: 32)

            VStack(alignment: .leading, spacing: 2) {
                Text(result.value)
                    .font(.system(size: 16, weight: .medium))
                Text(result.expression)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            HStack(spacing: 8) {
                Text("Calculator")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                if let s = shortcut {
                    KeyCap("⌘\(s)").opacity(isSelected ? 1.0 : 0.55)
                }
            }
        }
        .padding(.horizontal, PanelMetrics.rowHorizontalPadding)
        .padding(.vertical, PanelMetrics.rowVerticalPadding)
        .background(SelectionBackground(isSelected: isSelected))
    }
}

private struct AppRow: View {
    let app: MatchedAppDto
    let isSelected: Bool
    let shortcut: Int?
    @Environment(AppState.self) private var state
    @State private var icon: NSImage?

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if let icon {
                    Image(nsImage: icon).resizable().interpolation(.high)
                } else {
                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                        .fill(.secondary.opacity(0.15))
                }
            }
            .frame(width: 32, height: 32)

            Text(app.name)
                .font(.system(size: 14))

            Spacer(minLength: 8)

            HStack(spacing: 8) {
                Text("Application")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                if let s = shortcut {
                    KeyCap("⌘\(s)").opacity(isSelected ? 1.0 : 0.55)
                }
            }
        }
        .padding(.horizontal, PanelMetrics.rowHorizontalPadding)
        .padding(.vertical, PanelMetrics.rowVerticalPadding)
        .background(SelectionBackground(isSelected: isSelected))
        .task(id: app.path) {
            // Always reload — never trust a pre-existing icon, since
            // SwiftUI may have reused this view's @State for a different app.
            icon = await state.iconImage(app.path)
        }
    }
}

struct KeyCap: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.system(size: 10, weight: .medium, design: .rounded))
            .padding(.horizontal, 5)
            .padding(.vertical, 1.5)
            .background(
                RoundedRectangle(cornerRadius: 4, style: .continuous)
                    .fill(Color.gray.opacity(0.18))
            )
            .foregroundStyle(.secondary)
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
