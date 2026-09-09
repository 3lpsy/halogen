import SwiftUI

/// Generic read-only key→value page over any envelope endpoint — powers the
/// View Config and metadata screens without per-surface DTOs.
struct JSONKeyValueView: View {
    let title: String
    let core: HalogenCore
    let path: String
    /// When set, rows render local-first from this LocalStore key and every
    /// successful fetch re-snapshots it (the metadata screens — web renders
    /// those from the cached pool). Leave nil for genuinely network-only
    /// surfaces like View Config (network-only on the web too).
    var cacheKey: String?

    @State private var values: [(String, String)] = []
    @State private var error: String?

    var body: some View {
        Group {
            if let error {
                ContentUnavailableView {
                    Label("Couldn't load", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error).font(.footnote.monospaced())
                }
            } else if values.isEmpty {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(values, id: \.0) { key, value in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(key).font(.caption).foregroundStyle(.secondary)
                        Text(value).font(.callout.monospaced())
                    }
                    .padding(.vertical, 1)
                }
                .listStyle(.plain)
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        if values.isEmpty, let cacheKey, let store = core.store,
            let cached = await store.load([CachedRow].self, key: cacheKey)
        {
            values = cached.map { ($0.key, $0.value) }
        }
        do {
            let dict = try await core.rawJSON(path)
            values = dict.map {
                (
                    $0.key, Self.render($0.value)
                )
            }
            .sorted { $0.0 < $1.0 }
            error = nil
            if let cacheKey {
                let snapshot = values.map { CachedRow(key: $0.0, value: $0.1) }
                await core.store?.save(snapshot, key: cacheKey)
            }
        } catch {
            if values.isEmpty { self.error = FriendlyError.message(error) }
        }
    }

    /// The cached row shape (tuples aren't Codable).
    private struct CachedRow: Codable {
        let key: String
        let value: String
    }

    private static func render(_ value: Any) -> String {
        if value is NSNull { return "—" }
        if let s = value as? String { return s.isEmpty ? "\"\"" : s }
        if let n = value as? NSNumber {
            // Scalars must not reach JSONSerialization: a non-collection
            // top-level object raises an ObjC exception `try?` can't catch
            // (the View Config crash — every config has ports/flags).
            if n === kCFBooleanTrue { return "true" }
            if n === kCFBooleanFalse { return "false" }
            return n.stringValue
        }
        if JSONSerialization.isValidJSONObject(value),
            let data = try? JSONSerialization.data(
                withJSONObject: value, options: [.sortedKeys]),
            let s = String(data: data, encoding: .utf8)
        {
            return s
        }
        return "\(value)"
    }
}
