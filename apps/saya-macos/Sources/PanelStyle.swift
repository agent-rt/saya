import SwiftUI

/// Reusable visual primitives shared by Launcher / Clipboard panels.
/// Keeps spacing, materials and selection chrome consistent across views.

enum PanelMetrics {
    static let cornerRadius: CGFloat = 16
    static let rowVerticalPadding: CGFloat = 8
    static let rowHorizontalPadding: CGFloat = 16
    static let searchVerticalPadding: CGFloat = 14
    static let searchHorizontalPadding: CGFloat = 16
    static let searchFontSize: CGFloat = 22
    static let footerHeight: CGFloat = 32
}

/// Translucent rounded container — root of every panel SwiftUI tree.
struct PanelChrome<Content: View>: View {
    @ViewBuilder var content: () -> Content

    var body: some View {
        content()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.regularMaterial)
            .clipShape(RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: PanelMetrics.cornerRadius, style: .continuous)
                    .stroke(.white.opacity(0.08), lineWidth: 1)
            )
    }
}

/// Spotlight-style search bar with a leading magnifying glass icon.
/// `onCmdDigit(1..9) -> Bool` is consulted before the keystroke goes into
/// the text field; return `true` if the digit was consumed.
struct PanelSearchField<Trailing: View>: View {
    let placeholder: String
    @Binding var text: String
    @FocusState.Binding var focused: Bool
    let onSubmit: () -> Void
    let onUp: () -> Void
    let onDown: () -> Void
    var onCmdDigit: ((Int) -> Bool)? = nil
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 17, weight: .regular))
                .foregroundStyle(.tertiary)

            TextField(placeholder, text: $text, prompt: Text(placeholder).foregroundStyle(.tertiary))
                .textFieldStyle(.plain)
                .font(.system(size: PanelMetrics.searchFontSize, weight: .regular))
                .focused($focused)
                .onKeyPress(.upArrow) { onUp(); return .handled }
                .onKeyPress(.downArrow) { onDown(); return .handled }
                .onKeyPress { press in
                    guard let handler = onCmdDigit,
                          press.modifiers.contains(.command),
                          let digit = press.characters.first?.wholeNumberValue,
                          (1...9).contains(digit)
                    else { return .ignored }
                    return handler(digit) ? .handled : .ignored
                }
                .onSubmit(onSubmit)

            trailing()
        }
        .padding(.horizontal, PanelMetrics.searchHorizontalPadding)
        .padding(.vertical, PanelMetrics.searchVerticalPadding)
    }
}

extension PanelSearchField where Trailing == EmptyView {
    init(
        placeholder: String,
        text: Binding<String>,
        focused: FocusState<Bool>.Binding,
        onSubmit: @escaping () -> Void,
        onUp: @escaping () -> Void,
        onDown: @escaping () -> Void,
        onCmdDigit: ((Int) -> Bool)? = nil
    ) {
        self.placeholder = placeholder
        self._text = text
        self._focused = focused
        self.onSubmit = onSubmit
        self.onUp = onUp
        self.onDown = onDown
        self.onCmdDigit = onCmdDigit
        self.trailing = { EmptyView() }
    }
}

/// Bottom action bar with keycap hints (↵, ⎋, …). Lives at the panel's
/// bottom edge; uses the same material as the panel itself so it blends.
struct PanelFooter: View {
    let hints: [Hint]
    struct Hint: Identifiable {
        let id = UUID()
        let key: String
        let label: String
    }

    var body: some View {
        HStack(spacing: 14) {
            Spacer()
            ForEach(hints) { h in
                HStack(spacing: 5) {
                    Text(h.key)
                        .font(.system(size: 11, weight: .medium, design: .rounded))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(Color.gray.opacity(0.18))
                        )
                        .foregroundStyle(.secondary)
                    Text(h.label)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.horizontal, PanelMetrics.rowHorizontalPadding)
        .frame(height: PanelMetrics.footerHeight)
        .overlay(alignment: .top) {
            Divider().opacity(0.5)
        }
    }
}

/// Full-bleed selection background (no inset rounding) — matches Raycast,
/// plus a thin accent stripe on the leading edge of the selected row.
struct SelectionBackground: View {
    let isSelected: Bool
    var body: some View {
        ZStack(alignment: .leading) {
            if isSelected {
                Rectangle()
                    .fill(Color.accentColor.opacity(0.18))
                Rectangle()
                    .fill(Color.accentColor)
                    .frame(width: 3)
            }
        }
    }
}
