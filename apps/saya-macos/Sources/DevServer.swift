import AppKit
import Foundation
import Network

/// Local JSON-RPC control plane — CDP-style for Saya.
///
/// Listens on tcp://127.0.0.1:7896 by default. Speaks newline-delimited JSON
/// in both directions:
///
///     > {"id":1,"method":"panel.open","params":{"kind":"launcher"}}\n
///     < {"id":1,"result":null}\n
///
/// Methods exposed:
///   ping
///   panel.open       {kind: "launcher" | "clipboard"}
///   panel.close      {kind: "launcher" | "clipboard"}
///   panel.toggle     {kind: "launcher" | "clipboard"}
///   input.set        {query: String}            — sets launcher query, runs match, returns when results land
///   input.submit                                  — executes selected launcher item, closes panel
///   launcher.reset                                — clears query + selection
///   launcher.snapshot                             — returns {query, selected, items}
///   clipboard.snapshot                            — returns recent[]
///   state                                         — returns full FFI status
@MainActor
final class DevServer {
    private weak var state: AppState?
    private weak var panels: PanelController?
    private var listener: NWListener?

    /// Per-connection event subscriptions. Stored as a wrapper so we can hold
    /// strong refs to the connection while it's subscribed.
    private final class Subscription {
        let conn: NWConnection
        var patterns: Set<String>
        init(conn: NWConnection, patterns: Set<String>) {
            self.conn = conn
            self.patterns = patterns
        }
    }
    private var subscriptions: [ObjectIdentifier: Subscription] = [:]

    init(state: AppState, panels: PanelController) {
        self.state = state
        self.panels = panels
        // Wire event broadcasts. These closures are called from PanelController
        // / AppState whenever something interesting happens; we fan them out
        // to subscribed connections.
        state.eventSink = { [weak self] type, payload in
            self?.broadcast(type: type, payload: payload)
        }
        panels.eventSink = { [weak self] type, payload in
            self?.broadcast(type: type, payload: payload)
        }
    }

    func start(port: UInt16 = 7896) {
        let params = NWParameters.tcp
        params.acceptLocalOnly = true
        params.allowLocalEndpointReuse = true
        guard let nwPort = NWEndpoint.Port(rawValue: port) else {
            Log.error("DevServer: invalid port \(port)")
            return
        }
        do {
            let listener = try NWListener(using: params, on: nwPort)
            listener.stateUpdateHandler = { st in
                if case .failed(let err) = st {
                    Log.error("DevServer listener failed: \(err)")
                }
            }
            listener.newConnectionHandler = { [weak self] conn in
                guard let self else { return }
                Task { @MainActor in self.accept(conn) }
            }
            listener.start(queue: .main)
            self.listener = listener
            Log.info("DevServer listening on tcp://127.0.0.1:\(port)")
        } catch {
            Log.error("DevServer: bind failed: \(error)")
        }
    }

    // MARK: - Connection handling

    private func accept(_ conn: NWConnection) {
        conn.stateUpdateHandler = { [weak self] st in
            switch st {
            case .cancelled, .failed:
                Task { @MainActor in
                    self?.subscriptions.removeValue(forKey: ObjectIdentifier(conn))
                }
            default:
                break
            }
        }
        conn.start(queue: .main)
        readLines(from: conn, buffer: Data())
    }

    private func readLines(from conn: NWConnection, buffer: Data) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            var buf = buffer
            if let data { buf.append(data) }
            while let nl = buf.firstIndex(of: 0x0a) {
                let line = buf.prefix(upTo: nl)
                buf.removeSubrange(...nl)
                if !line.isEmpty {
                    Task { @MainActor in
                        self.dispatch(line: Data(line), on: conn)
                    }
                }
            }
            if isComplete || error != nil {
                conn.cancel()
                return
            }
            self.readLines(from: conn, buffer: buf)
        }
    }

    // MARK: - Dispatch

    private func dispatch(line: Data, on conn: NWConnection) {
        guard let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            sendError(conn, id: 0, message: "invalid JSON")
            return
        }
        let id = obj["id"] as? Int ?? 0
        let method = obj["method"] as? String ?? ""
        let params = obj["params"] as? [String: Any] ?? [:]
        Log.debug("dev RPC #\(id): \(method) \(params)")
        handle(method: method, params: params, id: id, conn: conn)
    }

    private func handle(method: String, params: [String: Any], id: Int, conn: NWConnection) {
        guard let state, let panels else {
            sendError(conn, id: id, message: "state not bound")
            return
        }
        switch method {
        case "ping":
            sendOk(conn, id: id, result: ["pong": true])

        case "panel.open":
            guard let kind = parseKind(params) else {
                sendError(conn, id: id, message: "missing or invalid 'kind'"); return
            }
            panels.show(kind)
            sendOk(conn, id: id, result: NSNull())

        case "panel.close":
            guard let kind = parseKind(params) else {
                sendError(conn, id: id, message: "missing or invalid 'kind'"); return
            }
            panels.close(kind)
            sendOk(conn, id: id, result: NSNull())

        case "panel.toggle":
            guard let kind = parseKind(params) else {
                sendError(conn, id: id, message: "missing or invalid 'kind'"); return
            }
            panels.toggle(kind)
            sendOk(conn, id: id, result: NSNull())

        case "input.set":
            let q = (params["query"] as? String) ?? ""
            Task { @MainActor in
                state.launcherQuery = q
                await state.refreshLauncher()
                self.sendOk(conn, id: id, result: NSNull())
            }

        case "input.submit":
            let what = state.executeLauncherSelection()
            panels.close(.launcher)
            sendOk(conn, id: id, result: ["executed": what])

        case "launcher.reset":
            state.resetLauncher()
            sendOk(conn, id: id, result: NSNull())

        case "launcher.snapshot":
            sendOk(conn, id: id, result: launcherSnapshot(state))

        case "clipboard.snapshot":
            sendOk(conn, id: id, result: clipboardSnapshot(state))

        case "state":
            sendOk(conn, id: id, result: stateSnapshot(state))

        case "settings.open":
            SettingsWindowController.shared.show(state: state)
            sendOk(conn, id: id, result: NSNull())

        case "app.activate":
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            sendOk(conn, id: id, result: NSNull())

        case "event.subscribe":
            let raw = (params["types"] as? [String]) ?? ["*"]
            let patterns = Set(raw)
            subscriptions[ObjectIdentifier(conn)] = Subscription(conn: conn, patterns: patterns)
            sendOk(conn, id: id, result: ["subscribed": Array(patterns)])

        case "event.unsubscribe":
            subscriptions.removeValue(forKey: ObjectIdentifier(conn))
            sendOk(conn, id: id, result: NSNull())

        default:
            sendError(conn, id: id, message: "unknown method: \(method)")
        }
    }

    // MARK: - Event broadcast

    /// Broadcast an event to every connection whose subscription patterns match.
    /// A pattern matches if it equals the event type, OR if it's a prefix of
    /// the type at a dot boundary, OR if it's literal "*".
    private func broadcast(type: String, payload: [String: Any]) {
        if subscriptions.isEmpty { return }
        var msg: [String: Any] = ["event": type]
        for (k, v) in payload { msg[k] = v }
        for sub in subscriptions.values where matches(type: type, patterns: sub.patterns) {
            sendJSON(sub.conn, msg)
        }
    }

    private func matches(type: String, patterns: Set<String>) -> Bool {
        if patterns.contains("*") || patterns.contains(type) {
            return true
        }
        for p in patterns where type.hasPrefix(p + ".") {
            return true
        }
        return false
    }

    private func parseKind(_ params: [String: Any]) -> PanelController.Kind? {
        guard let raw = params["kind"] as? String else { return nil }
        return PanelController.Kind(rawValue: raw)
    }

    // MARK: - Snapshots

    private func launcherSnapshot(_ state: AppState) -> [String: Any] {
        let items: [[String: Any]] = state.launcherItems.map { item in
            switch item {
            case .calc(let c):
                return ["kind": "calc",
                        "expression": c.expression,
                        "value": c.value]
            case .app(let app):
                return ["kind": "app",
                        "name": app.name,
                        "path": app.path,
                        "score": Int(app.score)]
            }
        }
        return [
            "query": state.launcherQuery,
            "selected": state.launcherSelected,
            "items": items,
        ]
    }

    private func clipboardSnapshot(_ state: AppState) -> [String: Any] {
        let items: [[String: Any]] = state.recent.prefix(50).map {
            ["id": Int($0.id),
             "content": $0.content,
             "ts": Int($0.createdAtUnixMs)]
        }
        return ["recent": items]
    }

    private func stateSnapshot(_ state: AppState) -> [String: Any] {
        var s: [String: Any] = [
            "launcherQuery": state.launcherQuery,
            "launcherSelected": state.launcherSelected,
            "launcherItemCount": state.launcherItems.count,
            "recentCount": state.recent.count,
        ]
        if let status = state.status {
            s["entries"]                  = Int(status.entryCount)
            s["entriesMissingVectors"]    = Int(status.entriesMissingVectors)
            s["embedderLoaded"]           = status.embedderLoaded
            s["clipboardMonitorRunning"]  = status.clipboardMonitorRunning
            s["embeddingFeatureCompiled"] = status.embeddingFeatureCompiled
        }
        return s
    }

    // MARK: - Send helpers

    private func sendOk(_ conn: NWConnection, id: Int, result: Any) {
        sendJSON(conn, ["id": id, "result": result])
    }

    private func sendError(_ conn: NWConnection, id: Int, message: String) {
        sendJSON(conn, ["id": id, "error": ["message": message]])
    }

    private func sendJSON(_ conn: NWConnection, _ obj: Any) {
        guard JSONSerialization.isValidJSONObject(obj),
              var data = try? JSONSerialization.data(withJSONObject: obj)
        else { return }
        data.append(0x0a) // newline-delimited
        conn.send(content: data, completion: .idempotent)
    }
}
