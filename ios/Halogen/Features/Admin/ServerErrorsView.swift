import SwiftUI

/// Server failure histories (admin): RSS sync + episode download errors —
/// reached from Settings, like the web.
struct ServerErrorsView: View {
    let core: HalogenCore

    @State private var data: ServerErrorsData?
    @State private var error: String?

    var body: some View {
        Group {
            if let error {
                ContentUnavailableView {
                    Label("Couldn't load errors", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error).font(.footnote.monospaced())
                }
            } else if let data {
                if data.rss_sync.isEmpty && data.episode_downloads.isEmpty {
                    ContentUnavailableView(
                        "No server errors",
                        systemImage: "checkmark.seal",
                        description: Text("Feed syncs and downloads are healthy.")
                    )
                } else {
                    List {
                        if !data.rss_sync.isEmpty {
                            Section("Feed sync") {
                                ForEach(data.rss_sync, id: \.id) { e in
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(e.podcast_title ?? "Podcast #\(e.podcast_id)")
                                            .font(.subheadline.weight(.medium))
                                        Text(e.reason)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                        Text(e.created_at, format: .dateTime.month().day().hour().minute())
                                            .font(.caption2)
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }
                        }
                        if !data.episode_downloads.isEmpty {
                            Section("Episode downloads") {
                                ForEach(data.episode_downloads, id: \.id) { e in
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(e.episode_title ?? "Episode #\(e.episode_id)")
                                            .font(.subheadline.weight(.medium))
                                        Text(e.reason)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                        Text(e.created_at, format: .dateTime.month().day().hour().minute())
                                            .font(.caption2)
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Server errors")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            data = try await core.serverErrors()
            error = nil
        } catch {
            if data == nil { self.error = FriendlyError.message(error) }
        }
    }
}
