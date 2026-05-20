import AppKit
import SwiftUI

struct LauncherView: View {
    var onDismiss: () -> Void = {}

    @Environment(AppState.self) private var state
    @State private var query: String = ""
    @State private var apps: [MatchedAppDto] = []
    @State private var calc: CalcResult?
    @State private var selected: Int = 0
    @FocusState private var focused: Bool

    private var items: [LauncherResult] {
        var out: [LauncherResult] = []
        if let calc { out.append(.calc(calc)) }
        out.append(contentsOf: apps.map { .app($0) })
        return out
    }

    var body: some View {
        PanelChrome {
            VStack(spacing: 0) {
                PanelSearchField(
                    placeholder: "Search applications or calculate…",
                    text: $query,
                    focused: $focused,
                    onSubmit: execute,
                    onUp: { moveSelection(-1) },
                    onDown: { moveSelection(1) },
                    onCmdDigit: { n in
                        let idx = n - 1
                        guard idx < items.count else { return false }
                        selected = idx
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
        .task(id: query) {
            // Calculator runs synchronously and is essentially free; do it
            // before the FFI roundtrip so the calc row is up the moment the
            // user finishes typing.
            calc = Calculator.evaluate(query).map {
                CalcResult(expression: query.trimmingCharacters(in: .whitespaces),
                           value: Calculator.format($0))
            }
            let limit: UInt32 = query.isEmpty ? 50 : 12
            let r = await state.matchApps(query, limit: limit)
            if Task.isCancelled { return }
            apps = r
            selected = 0
        }
        .onAppear { focused = true }
    }

    private var footerHints: [PanelFooter.Hint] {
        let primary: PanelFooter.Hint
        if case .calc = items.indices.contains(selected) ? items[selected] : nil {
            primary = .init(key: "↵", label: "Copy result")
        } else {
            primary = .init(key: "↵", label: "Open")
        }
        return [primary, .init(key: "⎋", label: "Dismiss")]
    }

    @ViewBuilder
    private var content: some View {
        if items.isEmpty {
            if !query.isEmpty {
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
                        ForEach(Array(items.enumerated()), id: \.offset) { idx, item in
                            row(for: item, at: idx)
                                .id(idx)
                                .contentShape(Rectangle())
                                .onHover { hovering in
                                    if hovering { selected = idx }
                                }
                                .onTapGesture {
                                    selected = idx
                                    execute()
                                }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .onChange(of: selected) { _, new in
                    withAnimation(.easeOut(duration: 0.08)) {
                        proxy.scrollTo(new, anchor: .center)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func row(for item: LauncherResult, at idx: Int) -> some View {
        let shortcut = idx < 9 ? idx + 1 : nil
        let selected = idx == selected
        switch item {
        case .calc(let c):
            CalcRow(result: c, isSelected: selected, shortcut: shortcut)
        case .app(let app):
            AppRow(app: app, isSelected: selected, shortcut: shortcut)
        }
    }

    private func moveSelection(_ delta: Int) {
        guard !items.isEmpty else { return }
        selected = (selected + delta).clamped(to: 0...(items.count - 1))
    }

    private func execute() {
        guard items.indices.contains(selected) else { return }
        switch items[selected] {
        case .calc(let c):
            state.copyToPasteboard(c.value)
        case .app(let app):
            state.launch(app.path)
        }
        onDismiss()
    }
}

// MARK: - Result model

enum LauncherResult {
    case calc(CalcResult)
    case app(MatchedAppDto)
}

struct CalcResult: Equatable {
    let expression: String
    let value: String
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
            if icon == nil {
                icon = await state.iconImage(app.path)
            }
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
