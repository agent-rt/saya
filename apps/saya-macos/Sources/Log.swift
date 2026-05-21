import Foundation

/// Swift-side logger that funnels into the Rust tracing pipeline, so the
/// SwiftUI shell's events land in the same `~/Library/Logs/Saya/saya.log`
/// file as the Rust core. Targets `saya_ui` so log filters can distinguish:
///     SAYA_LOG=saya_ui=debug,saya_core=info
enum Log {
    static func info(_ message: @autoclosure () -> String)  { emit("info",  message()) }
    static func warn(_ message: @autoclosure () -> String)  { emit("warn",  message()) }
    static func error(_ message: @autoclosure () -> String) { emit("error", message()) }
    static func debug(_ message: @autoclosure () -> String) { emit("debug", message()) }

    private static func emit(_ level: String, _ message: String) {
        logFromSwift(level: level, message: message)
    }
}
