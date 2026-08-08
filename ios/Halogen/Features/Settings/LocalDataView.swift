import SwiftUI

/// Local Data (the web's cache-control page, native): everything stored on
/// this device with live size/count stats, each category individually
/// deletable — caches, prefs, sync ops, audio, artwork, other accounts, logs.
struct LocalDataView: View {
    let core: HalogenCore

    @State private var stats = LocalDataStats()
    @State private var message: String?
    @State private var confirmEmbedded = false
    @State private var confirmDeleteAll = false
    @State private var busy = false

    var body: some View {
        List {
            // Result feedback FIRST — at the bottom of this long list it sat
            // below the fold, so purges looked unacknowledged.
            if let message {
                Section { Text(message).foregroundStyle(.secondary) }
            }

            Section {
                purgeRow(
                    "Cached lists",
                    detail: "\(stats.contentCount) items · \(Self.size(stats.contentBytes))",
                    icon: "internaldrive"
                ) {
                    await core.store?.remove(keys: stats.contentKeys)
                    core.remountModels()
                    note("Cleared the content cache")
                }
                purgeRow(
                    "View settings",
                    detail: "\(stats.viewSettingsCount) items",
                    icon: "slider.horizontal.3"
                ) {
                    await core.store?.remove(keys: stats.viewSettingsKeys)
                    core.remountModels()
                    note("Reset view settings")
                }
                purgeRow(
                    "Preferences",
                    detail: "\(stats.prefsCount) items",
                    icon: "gearshape"
                ) {
                    await core.store?.remove(keys: stats.prefsKeys)
                    core.remountModels()
                    note("Reset preferences")
                }
                purgeRow(
                    "Pending sync queue",
                    detail: "\(stats.outboxCount) ops",
                    icon: "arrow.triangle.2.circlepath"
                ) {
                    await core.outbox?.clearAll()
                    note("Discarded the pending sync queue")
                }
                purgeRow(
                    "Device audio",
                    detail: "\(stats.audioCount) files · \(Self.size(stats.audioBytes))",
                    icon: "arrow.down.circle"
                ) {
                    core.models?.device.removeAll()
                    note("Deleted device audio")
                }
                purgeRow(
                    "Artwork cache (memory)",
                    detail: "in-memory",
                    icon: "photo"
                ) {
                    ArtLoader.shared.configure(token: core.apiToken)
                    note("Cleared artwork cache")
                }
                purgeRow(
                    "Device log",
                    detail: "\(DeviceLog.shared.entries.count) entries",
                    icon: "doc.text"
                ) {
                    DeviceLog.shared.clear()
                    note("Cleared the device log")
                }
            } header: {
                Text("This account")
            }

            Section {
                purgeRow(
                    "Other accounts' data",
                    detail: "\(stats.otherAccounts) accounts · \(Self.size(stats.otherBytes))",
                    icon: "person.2"
                ) {
                    LocalDataStats.removeOtherNamespaces(active: core.account?.namespace)
                    note("Deleted other accounts' local data")
                }
            } header: {
                Text("Other accounts")
            }

            Section {
                Button("Delete all local data…", role: .destructive) {
                    confirmDeleteAll = true
                }
                .disabled(busy)
            } footer: {
                Text(
                    "Clears every cache, setting, pending sync op, and downloaded file for all accounts on this device. The embedded server library below is separate."
                )
            }

            Section {
                HStack {
                    Label("Embedded server data", systemImage: "externaldrive.badge.xmark")
                    Spacer()
                    Text(Self.size(stats.embeddedBytes))
                        .foregroundStyle(.secondary)
                        .font(.callout)
                }
                Button("Delete embedded server…", role: .destructive) {
                    confirmEmbedded = true
                }
                .disabled(busy || stats.embeddedBytes == 0)
            } header: {
                Text("Embedded server")
            } footer: {
                Text(
                    "Deletes this device's entire library — database, media files, and users. Remote servers are unaffected."
                )
            }

        }
        .navigationTitle("Local data")
        .navigationBarTitleDisplayMode(.inline)
        .task { await reload() }
        .refreshable { await reload() }
        .confirmationDialog(
            "Delete the embedded server?", isPresented: $confirmEmbedded
        ) {
            Button("Delete everything", role: .destructive) {
                Task { await deleteEmbedded() }
            }
        } message: {
            Text("The database, media, and every device user are permanently removed.")
        }
        .confirmationDialog(
            "Delete all local data?", isPresented: $confirmDeleteAll
        ) {
            Button("Delete all", role: .destructive) {
                Task { await deleteAll() }
            }
        } message: {
            Text(
                "Caches, view settings, preferences, pending sync ops, device audio, and other accounts' data are permanently removed. The embedded server library is not touched."
            )
        }
    }

    private func purgeRow(
        _ title: String, detail: String, icon: String,
        action: @escaping () async -> Void
    ) -> some View {
        HStack {
            Label(title, systemImage: icon)
            Spacer()
            Text(detail).foregroundStyle(.secondary).font(.callout)
            Button(role: .destructive) {
                Task {
                    await action()
                    await reload()
                }
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
        }
    }

    private func reload() async {
        stats = await LocalDataStats.collect(core: core)
    }

    private func note(_ text: String) {
        message = text
    }

    /// The one-tap full reset (everything except the embedded library).
    /// Goes through the models so live state — device download statuses
    /// included — resets with the files.
    private func deleteAll() async {
        busy = true
        defer { busy = false }
        await core.store?.remove(
            keys: stats.contentKeys + stats.viewSettingsKeys + stats.prefsKeys)
        await core.outbox?.clearAll()
        core.models?.device.removeAll()
        ArtLoader.shared.configure(token: core.apiToken)
        DeviceLog.shared.clear()
        LocalDataStats.removeOtherNamespaces(active: core.account?.namespace)
        core.remountModels()
        note("Deleted all local data")
        await reload()
    }

    private func deleteEmbedded() async {
        busy = true
        defer { busy = false }
        do {
            try await core.destroyEmbeddedServer()
            note("Embedded server deleted")
            await reload()
        } catch {
            note("Delete failed: \(error)")
        }
    }

    private static func size(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }
}

/// The census behind the Local Data screen.
struct LocalDataStats {
    var contentKeys: [String] = []
    var viewSettingsKeys: [String] = []
    var prefsKeys: [String] = []
    var contentCount = 0
    var contentBytes: UInt64 = 0
    var viewSettingsCount = 0
    var prefsCount = 0
    var outboxCount = 0
    var audioCount = 0
    var audioBytes: UInt64 = 0
    var otherAccounts = 0
    var otherBytes: UInt64 = 0
    var embeddedBytes: UInt64 = 0

    static let prefKeys: Set<String> = ["nav-config", "swipe-prefs", "client-prefs"]

    @MainActor
    static func collect(core: HalogenCore) async -> LocalDataStats {
        var stats = LocalDataStats()
        if let store = core.store {
            let keys = await store.listKeys()
            for key in keys {
                if prefKeys.contains(key) {
                    stats.prefsKeys.append(key)
                } else if key.hasPrefix("listquery-") || key.hasSuffix("-scroll-anchor") {
                    stats.viewSettingsKeys.append(key)
                } else if key == "outbox" {
                    // counted below from the live outbox
                } else {
                    stats.contentKeys.append(key)
                }
            }
            stats.contentCount = stats.contentKeys.count
            stats.contentBytes = await store.bytes(forKeys: stats.contentKeys)
            stats.viewSettingsCount = stats.viewSettingsKeys.count
            stats.prefsCount = stats.prefsKeys.count
        }
        stats.outboxCount = await core.outbox?.pendingCount ?? 0
        if let device = core.models?.device {
            let s = device.stats
            stats.audioCount = s.count
            stats.audioBytes = s.bytes
        }
        let (others, otherBytes) = otherNamespaceStats(active: core.account?.namespace)
        stats.otherAccounts = others
        stats.otherBytes = otherBytes
        stats.embeddedBytes = directoryBytes(embeddedRoot())
        return stats
    }

    // MARK: - filesystem census helpers

    static func clientRoot() -> URL {
        let support = try! FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true)
        return support.appendingPathComponent("halogen-client", isDirectory: true)
    }

    static func embeddedRoot() -> URL {
        let support = try! FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true)
        return support.appendingPathComponent("halogen-server", isDirectory: true)
    }

    static func otherNamespaceStats(active: String?) -> (Int, UInt64) {
        let root = clientRoot()
        let names =
            ((try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? [])
            .filter { $0 != active }
        let bytes = names.reduce(UInt64(0)) {
            $0 + directoryBytes(root.appendingPathComponent($1, isDirectory: true))
        }
        return (names.count, bytes)
    }

    static func removeOtherNamespaces(active: String?) {
        let root = clientRoot()
        let names =
            ((try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? [])
            .filter { $0 != active }
        for name in names {
            try? FileManager.default.removeItem(
                at: root.appendingPathComponent(name, isDirectory: true))
        }
    }

    static func directoryBytes(_ url: URL) -> UInt64 {
        guard
            let enumerator = FileManager.default.enumerator(
                at: url, includingPropertiesForKeys: [.fileSizeKey])
        else { return 0 }
        var total: UInt64 = 0
        for case let file as URL in enumerator {
            total += UInt64((try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
        }
        return total
    }
}
