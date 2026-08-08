import SwiftUI

/// This device's captured app events (the in-app DeviceLog ring) — the
/// native `/logs/device`: capture toggle + level threshold applied live,
/// search filter, export, and a clear that also wipes persisted storage.
struct DeviceLogsView: View {
    let core: HalogenCore

    @State private var log = DeviceLog.shared
    @State private var query = ""
    @State private var exportFile: URL?

    private var filtered: [DeviceLog.Entry] {
        let all = log.entries.reversed()
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return Array(all) }
        // Message OR source tag (web: search matches msg + target).
        return all.filter {
            $0.message.lowercased().contains(q)
                || ($0.source?.lowercased().contains(q) ?? false)
        }
    }

    var body: some View {
        List {
            Section("Capture") {
                Toggle(
                    "Enable device logs",
                    isOn: Binding(
                        get: { log.enabled },
                        set: { log.enabled = $0 }
                    )
                )
                Picker(
                    "Log level",
                    selection: Binding(
                        get: { log.threshold },
                        set: { log.threshold = $0 }
                    )
                ) {
                    ForEach(DeviceLog.Level.allCases) { level in
                        Text(level.label).tag(level)
                    }
                }
            }

            Section {
                TextField("Search logs…", text: $query)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                if let exportFile {
                    ShareLink(item: exportFile) {
                        Label(exportFile.lastPathComponent, systemImage: "doc.badge.arrow.up")
                    }
                }
            }

            Section {
                if filtered.isEmpty {
                    Text(
                        query.trimmingCharacters(in: .whitespaces).isEmpty
                            ? (log.enabled
                                ? "No logs captured yet."
                                : "Capture disabled — enable device logs to record events.")
                            : "No logs match your search."
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                } else {
                    ForEach(filtered) { entry in
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                Text(entry.level.rawValue.uppercased())
                                    .font(.caption2.weight(.bold))
                                    .foregroundStyle(color(entry.level))
                                Text(entry.at, format: .dateTime.hour().minute().second())
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                // Rust tracing target — distinguishes embedded-
                                // server lines from app lines (web: the target
                                // column on /logs/device).
                                if let source = entry.source {
                                    Text(source)
                                        .font(.caption2.monospaced())
                                        .foregroundStyle(.tertiary)
                                        .lineLimit(1)
                                }
                            }
                            Text(entry.message).font(.caption.monospaced())
                        }
                        .padding(.vertical, 1)
                    }
                }
            } header: {
                Text(log.enabled ? "Capturing • \(log.entries.count) lines" : "Capture disabled")
            }
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button("Clear") {
                    log.clear()
                    exportFile = nil
                }
                .disabled(log.entries.isEmpty)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    export()
                } label: {
                    Image(systemName: "square.and.arrow.up")
                }
                .disabled(log.entries.isEmpty)
            }
        }
    }

    private func export() {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("halogen-device-logs.txt")
        do {
            try log.exportText().write(to: file, atomically: true, encoding: .utf8)
            exportFile = file
        } catch {
            ToastCenter.shared.error("Export failed: \(error)")
        }
    }

    private func color(_ level: DeviceLog.Level) -> Color {
        switch level {
        case .info: return .secondary
        case .warn: return .orange
        case .error: return .red
        }
    }
}
