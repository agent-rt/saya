import KeyboardShortcuts

extension KeyboardShortcuts.Name {
    static let toggleLauncher = Self("toggleLauncher", default: .init(.space, modifiers: [.option]))
    static let toggleClipboard = Self("toggleClipboard", default: .init(.v, modifiers: [.option, .shift]))
}
