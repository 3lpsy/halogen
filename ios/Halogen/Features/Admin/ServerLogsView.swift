import SwiftUI

/// Server log tail (admin) — the embedded/remote server's own application
/// log, parsed into the Device Logs presentation (level badge, time, source,
/// message), newest first, with search and manual refresh.
struct ServerLogsView: View {
    let core: HalogenCore

    @State private var logs: ServerLogsData?
    @State private var error: String?
    @State private var query = ""
    @State private var loading = false

    /// One parsed tracing line. Raw lines are ANSI-coded
    /// `<ISO8601> LEVEL <file>:<line>: <message>`; anything that doesn't
    /// parse renders whole as the message (never drop a line).
    private struct Line: Identifiable {
        let id: Int
        let time: String?
        let level: String?
        let source: String?
        let message: String
    }

    var body: some View {
        Group {
            if let error {
                ContentUnavailableView {
                    Label("Couldn't load logs", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error).font(.footnote.monospaced())
                }
            } else if let logs {
                if logs.lines.isEmpty {
                    ContentUnavailableView(
                        "No log lines",
                        systemImage: "doc.text",
                        description: Text(
                            logs.path == nil
                                ? "No log file is configured. Local Only logs are available in Device logs."
                                : "The log file is empty.")
                    )
                } else {
                    logList
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { await load() }
                } label: {
                    if loading {
                        ProgressView()
                    } else {
                        Image(systemName: "arrow.clockwise")
                    }
                }
                .disabled(loading)
                .accessibilityLabel("Refresh")
            }
        }
        .task { await load() }
        .refreshable { await load() }
    }

    private var logList: some View {
        Form {
            Section {
                TextField("Search logs…", text: $query)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            }
            Section {
                if filtered.isEmpty {
                    Text("No lines match your search.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(filtered) { line in
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                if let level = line.level {
                                    Text(level)
                                        .font(.caption2.weight(.bold))
                                        .foregroundStyle(color(level))
                                }
                                if let time = line.time {
                                    Text(time)
                                        .font(.caption2)
                                        .foregroundStyle(.tertiary)
                                }
                                if let source = line.source {
                                    Text(source)
                                        .font(.caption2.monospaced())
                                        .foregroundStyle(.tertiary)
                                        .lineLimit(1)
                                }
                            }
                            Text(line.message).font(.caption.monospaced())
                        }
                        .padding(.vertical, 1)
                    }
                }
            } header: {
                // Newest first, like the web's tail; the path names the file.
                Text("\(filtered.count) of \(parsed.count) lines • newest first")
            }
        }
    }

    // MARK: - parsing

    private var parsed: [Line] {
        let lines = logs?.lines ?? []
        return lines.enumerated().reversed().map { idx, raw in
            Self.parse(raw, id: idx)
        }
    }

    private var filtered: [Line] {
        let trimmed = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !trimmed.isEmpty else { return parsed }
        return parsed.filter {
            $0.message.lowercased().contains(trimmed)
                || ($0.source?.lowercased().contains(trimmed) ?? false)
                || ($0.level?.lowercased().contains(trimmed) ?? false)
        }
    }

    /// `2026-07-29T05:21:48.329717Z  INFO crates/server/src/x.rs:146: msg`
    /// (with ANSI color codes) → level/time/source/message.
    private static func parse(_ raw: String, id: Int) -> Line {
        let clean = raw.replacingOccurrences(
            of: "\u{1B}\\[[0-9;]*m", with: "", options: .regularExpression)
        let parts = clean.split(separator: " ", omittingEmptySubsequences: true)
        guard parts.count >= 3,
            parts[0].contains("T"),
            ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"].contains(String(parts[1]))
        else {
            return Line(id: id, time: nil, level: nil, source: nil, message: clean)
        }
        // hh:mm:ss out of the ISO timestamp.
        let time = parts[0].split(separator: "T").last.map {
            String($0.prefix(8))
        }
        let level = String(parts[1])
        // Source is `path/file.rs:line:` — shorten to its last two segments.
        var source: String?
        var messageStart = 2
        if parts.count > 3, parts[2].hasSuffix(":") {
            let full = String(parts[2].dropLast())
            source = full.split(separator: "/").suffix(2).joined(separator: "/")
            messageStart = 3
        }
        let message = parts[messageStart...].joined(separator: " ")
        return Line(id: id, time: time, level: level, source: source, message: message)
    }

    private func color(_ level: String) -> Color {
        switch level {
        case "ERROR": return .red
        case "WARN": return .orange
        case "INFO": return .blue
        default: return .secondary
        }
    }

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            logs = try await core.serverLogs()
            error = nil
        } catch {
            if logs == nil { self.error = FriendlyError.message(error) }
        }
    }
}
