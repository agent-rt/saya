import SwiftUI

struct ClipboardView: View {
    var onDismiss: () -> Void = {}

    @Environment(AppState.self) private var state
    @State private var query: String = ""
    @State private var semantic: Bool = false
    @State private var hits: [SearchHitDto] = []
    @State private var selected: Int = 0
    @FocusState private var focused: Bool

    private var items: [Item] {
        if query.isEmpty {
            return state.recent.map { Item(id: $0.id, content: $0.content, ts: $0.createdAtUnixMs) }
        } else {
            return hits.map { Item(id: $0.id, content: $0.content, ts: $0.createdAtUnixMs) }
        }
    }

    var body: some View {
        PanelChrome {
            VStack(spacing: 0) {
                PanelSearchField(
                    placeholder: "Search clipboard…",
                    text: $query,
                    focused: $focused,
                    onSubmit: copySelected,
                    onUp: { moveSelection(-1) },
                    onDown: { moveSelection(1) },
                    onCmdDigit: { n in
                        let idx = n - 1
                        guard idx < items.count else { return false }
                        selected = idx
                        copySelected()
                        return true
                    }
                ) {
                    SemanticToggle(isOn: $semantic)
                }

                Divider().opacity(0.4)

                content

                Divider().opacity(0.4)

                PanelFooter(hints: [
                    .init(key: "↵", label: "Copy & paste"),
                    .init(key: "⎋", label: "Dismiss"),
                ])
            }
        }
        .task(id: "\(query)|\(semantic)") {
            if query.isEmpty {
                await state.refreshRecent(50)
            } else {
                try? await Task.sleep(nanoseconds: 120_000_000)
                if Task.isCancelled { return }
                hits = await state.searchClipboard(query, semantic: semantic)
            }
            selected = 0
        }
        .onAppear { focused = true }
    }

    @ViewBuilder
    private var content: some View {
        if items.isEmpty {
            VStack {
                Spacer()
                if query.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "doc.on.clipboard")
                            .font(.largeTitle)
                            .foregroundStyle(.tertiary)
                        Text("Clipboard empty")
                            .foregroundStyle(.secondary)
                        Text("Enable the monitor in Settings to start capturing.")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                } else {
                    Text("No matches").font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                            ClipboardRow(
                                content: item.content,
                                ts: item.ts,
                                isSelected: idx == selected,
                                shortcut: idx < 9 ? idx + 1 : nil
                            )
                            .id(idx)
                            .contentShape(Rectangle())
                            .onHover { hovering in
                                if hovering { selected = idx }
                            }
                            .onTapGesture {
                                selected = idx
                                copySelected()
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

    private func moveSelection(_ delta: Int) {
        guard !items.isEmpty else { return }
        selected = (selected + delta).clamped(to: 0...(items.count - 1))
    }

    private func copySelected() {
        guard selected < items.count else { return }
        state.copyToPasteboard(items[selected].content)
        onDismiss()
    }
}

private struct Item: Identifiable {
    let id: Int64
    let content: String
    let ts: Int64
}

private struct ClipboardRow: View {
    let content: String
    let ts: Int64
    let isSelected: Bool
    let shortcut: Int?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "doc.on.clipboard")
                .font(.system(size: 14))
                .foregroundStyle(.tertiary)
                .frame(width: 22)
                .padding(.top, 2)

            VStack(alignment: .leading, spacing: 2) {
                Text(content)
                    .font(.system(size: 13))
                    .lineLimit(2)
                    .truncationMode(.tail)
                Text(timeAgo(ts))
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 8)

            HStack(spacing: 8) {
                Text("Paste")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                if let s = shortcut {
                    Text("⌘\(s)")
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1.5)
                        .background(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(Color.gray.opacity(0.18))
                        )
                        .foregroundStyle(.secondary)
                        .opacity(isSelected ? 1.0 : 0.55)
                }
            }
        }
        .padding(.horizontal, PanelMetrics.rowHorizontalPadding)
        .padding(.vertical, PanelMetrics.rowVerticalPadding)
        .background(SelectionBackground(isSelected: isSelected))
    }

    private func timeAgo(_ unixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(unixMs) / 1000)
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

private struct SemanticToggle: View {
    @Binding var isOn: Bool

    var body: some View {
        Button {
            isOn.toggle()
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "sparkles")
                    .font(.system(size: 11))
                Text("Semantic")
                    .font(.system(size: 11, weight: .medium))
            }
            .foregroundStyle(isOn ? Color.accentColor : .secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(isOn ? Color.accentColor.opacity(0.18) : Color.gray.opacity(0.15))
            )
        }
        .buttonStyle(.plain)
        .help("Enable semantic / vector search")
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
