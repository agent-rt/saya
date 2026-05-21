import Foundation

/// One row in the launcher panel — either an inline calculator result or
/// a matched application from the launcher index.
enum LauncherResult: Identifiable, Equatable {
    case calc(CalcResult)
    case app(MatchedAppDto)

    /// Stable identity used by SwiftUI's ForEach + ScrollViewReader so a
    /// row is destroyed (not reused) when its underlying app changes.
    /// Reusing rows by ordinal index caused @State (e.g. the cached icon)
    /// to leak between different apps — the "Chrome row showing some other
    /// app's icon" bug.
    var id: String {
        switch self {
        case .calc(let c): return "calc:\(c.expression)"
        case .app(let a):  return "app:\(a.path)"
        }
    }
}

struct CalcResult: Equatable, Sendable {
    let expression: String
    let value: String
}
