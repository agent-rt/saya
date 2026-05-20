import KeyboardShortcuts
import SwiftUI

struct SettingsView: View {
    @Environment(AppState.self) private var state
    @State private var monitorOn: Bool = false
    @State private var embedOn: Bool = false
    @State private var reindexing: Bool = false
    @State private var lastReindex: UInt32?

    var body: some View {
        Form {
            Section("Global shortcuts") {
                KeyboardShortcuts.Recorder("Launcher", name: .toggleLauncher)
                KeyboardShortcuts.Recorder("Clipboard", name: .toggleClipboard)
            }

            Section("Clipboard monitor") {
                Toggle("Capture clipboard history", isOn: $monitorOn)
                    .onChange(of: monitorOn) { _, isOn in
                        if isOn {
                            state.startClipboardMonitor(embed: embedOn)
                        } else {
                            state.stopClipboardMonitor()
                        }
                    }
                Toggle("Generate embeddings on capture", isOn: $embedOn)
                    .disabled(!monitorOn || !embeddingCompiled)
                    .onChange(of: embedOn) { _, _ in
                        // Restart monitor with new flag.
                        if monitorOn {
                            state.stopClipboardMonitor()
                            state.startClipboardMonitor(embed: embedOn)
                        }
                    }
                if !embeddingCompiled {
                    Text("Embedding feature not compiled into this build.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Section("Status") {
                if let s = state.status {
                    StatusRow(label: "Entries", value: "\(s.entryCount)")
                    StatusRow(label: "Missing vectors", value: "\(s.entriesMissingVectors)")
                    StatusRow(label: "Embedder loaded", value: s.embedderLoaded ? "yes" : "no")
                    StatusRow(label: "DB", value: s.dbPath, mono: true)
                } else {
                    Text("Loading…").foregroundStyle(.secondary)
                }
            }

            Section("Maintenance") {
                HStack {
                    Button {
                        Task {
                            reindexing = true
                            lastReindex = await state.reindex()
                            state.refreshStatus()
                            reindexing = false
                        }
                    } label: {
                        Text("Backfill embeddings for older entries")
                    }
                    .disabled(reindexing || !embeddingCompiled)
                    if reindexing { ProgressView().controlSize(.small) }
                }
                if let n = lastReindex {
                    Text("Processed \(n) entries.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Button("Unload embedder (free Metal memory)") {
                    state.unloadEmbedder()
                }
                .disabled(!embeddingCompiled)
            }
        }
        .formStyle(.grouped)
        .task {
            monitorOn = state.status?.clipboardMonitorRunning ?? false
        }
    }

    private var embeddingCompiled: Bool {
        state.status?.embeddingFeatureCompiled ?? false
    }
}

private struct StatusRow: View {
    let label: String
    let value: String
    var mono: Bool = false

    var body: some View {
        LabeledContent(label) {
            Text(value)
                .font(mono ? .system(.caption, design: .monospaced) : .body)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(.secondary)
        }
    }
}
