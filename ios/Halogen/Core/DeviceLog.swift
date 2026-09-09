import Foundation
import Observation

/// In-app diagnostic log ring (native counterpart of halogen-ui-logging's):
/// bounded at 5000 lines, persisted across launches, capture toggle + live
/// level threshold; rendered by the Device Logs page.
@MainActor
@Observable
final class DeviceLog {
    struct Entry: Identifiable, Codable {
        let id: UUID
        let at: Date
        let level: Level
        let message: String
        /// Rust tracing target for core-originated lines (`halogen_server`, …)
        /// — the web's /logs/device target column. `nil` for app-side lines.
        let source: String?

        init(at: Date, level: Level, message: String, source: String? = nil) {
            self.id = UUID()
            self.at = at
            self.level = level
            self.message = message
            self.source = source
        }
    }

    /// Severity, most-severe first — a line is captured when its severity is
    /// at or above the threshold (web: `level as u8 <= threshold`).
    enum Level: String, Codable, CaseIterable, Identifiable {
        case error
        case warn
        case info

        var id: String { rawValue }

        var severity: Int {
            switch self {
            case .error: return 0
            case .warn: return 1
            case .info: return 2
            }
        }

        /// Human label for the threshold picker (web: Level::label).
        var label: String {
            switch self {
            case .error: return "Error only"
            case .warn: return "Warn and above"
            case .info: return "Info and above (default)"
            }
        }
    }

    static let shared = DeviceLog()
    /// Web: device_log::CAP.
    private static let capacity = 5000
    private static let enabledKey = "device-log.enabled"
    private static let levelKey = "device-log.level"

    private(set) var entries: [Entry] = []

    /// Capture gate — when off, nothing new is recorded (web parity).
    var enabled: Bool {
        didSet {
            UserDefaults.standard.set(enabled, forKey: Self.enabledKey)
            pushCaptureToCore()
        }
    }

    /// Minimum severity captured into the ring.
    var threshold: Level {
        didSet {
            UserDefaults.standard.set(threshold.rawValue, forKey: Self.levelKey)
            pushCaptureToCore()
        }
    }

    private var saveTask: Task<Void, Never>?
    /// Drains the Rust core's tracing ring (the embedded server's only log
    /// surface) into this one. Started once the embedded server boots.
    private var corePumpTask: Task<Void, Never>?

    init() {
        let defaults = UserDefaults.standard
        enabled = defaults.object(forKey: Self.enabledKey) as? Bool ?? true
        threshold =
            defaults.string(forKey: Self.levelKey).flatMap(Level.init(rawValue:)) ?? .info
        loadPersisted()
    }

    func log(_ level: Level, _ message: String) {
        #if DEBUG
            print("devicelog[\(level.rawValue)]: \(message)")
        #endif
        guard enabled, level.severity <= threshold.severity else { return }
        append(Entry(at: Date(), level: level, message: message))
    }

    private func append(_ entry: Entry) {
        entries.append(entry)
        if entries.count > Self.capacity {
            entries.removeFirst(entries.count - Self.capacity)
        }
        scheduleSave()
    }

    // MARK: - Rust core capture (embedded-server tracing → this ring)

    /// Poll the Rust core's device-log ring (2s, matching the web's flush
    /// loop): drained embedded-server tracing lines land here tagged with
    /// their target. Idempotent.
    func startCorePump() {
        guard corePumpTask == nil else { return }
        pushCaptureToCore()
        corePumpTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard let self else { return }
                for line in drainDeviceLog() {
                    self.ingest(line)
                }
            }
        }
    }

    /// Mirror the capture toggle + threshold into the Rust ring so filtered
    /// lines are never buffered core-side (the Rust gate is authoritative;
    /// `ingest` re-applies it only for lines already in flight).
    private func pushCaptureToCore() {
        setDeviceLogCapture(enabled: enabled, level: threshold.rawValue.capitalized)
    }

    private func ingest(_ line: DeviceLogLine) {
        let level: Level
        switch line.level {
        case "Error": level = .error
        case "Warn": level = .warn
        // Debug/Trace have no native tier — the Rust filter caps server
        // crates at info, so this is a defensive fold, not a hot path.
        default: level = .info
        }
        guard enabled, level.severity <= threshold.severity else { return }
        append(
            Entry(
                at: Date(timeIntervalSince1970: Double(line.tsMs) / 1000),
                level: level,
                message: line.msg,
                source: line.target
            ))
    }

    nonisolated static func info(_ message: String) {
        Task { @MainActor in shared.log(.info, message) }
    }

    nonisolated static func warn(_ message: String) {
        Task { @MainActor in shared.log(.warn, message) }
    }

    nonisolated static func error(_ message: String) {
        Task { @MainActor in shared.log(.error, message) }
    }

    /// Drop the ring AND the persisted copy (cleared lines must not return
    /// on relaunch — web: logging::clear + store::clear).
    func clear() {
        entries.removeAll()
        saveTask?.cancel()
        if let url = Self.fileURL {
            try? FileManager.default.removeItem(at: url)
        }
    }

    /// Plain-text export, oldest first (web: logging::export_text).
    func exportText() -> String {
        let formatter = ISO8601DateFormatter()
        return
            entries
            .map {
                let source = $0.source.map { "\($0): " } ?? ""
                return
                    "\(formatter.string(from: $0.at)) \($0.level.rawValue.uppercased()) \(source)\($0.message)"
            }
            .joined(separator: "\n")
    }

    // MARK: - persistence (2s debounced flush, web parity)

    private static var fileURL: URL? {
        guard
            let dir = try? FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask,
                appropriateFor: nil, create: true)
        else { return nil }
        return dir.appendingPathComponent("device-log.json")
    }

    private func loadPersisted() {
        guard let url = Self.fileURL,
            let data = try? Data(contentsOf: url),
            let saved = try? JSONDecoder().decode([Entry].self, from: data)
        else { return }
        entries = Array(saved.suffix(Self.capacity))
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.persistRing()
        }
    }

    private func persistRing() {
        guard let url = Self.fileURL,
            let data = try? JSONEncoder().encode(entries)
        else { return }
        try? data.write(to: url, options: .atomic)
    }
}
