import SwiftUI

struct DiscoverEpisodeRow: View {
    let item: DiscoverEpisodeItem

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(item.title).font(.headline)
            Text(item.podcast_title).font(.caption).foregroundStyle(.secondary)
            if !item.discoverMetadata.isEmpty { Text(item.discoverMetadata).font(.caption).foregroundStyle(.secondary) }
            if !item.description.isEmpty {
                Text(HTMLText.preview(item.description)).font(.subheadline).foregroundStyle(.secondary).lineLimit(3)
            }
        }
        .padding(.vertical, 4)
    }

}

extension DiscoverEpisodeItem {
    var discoverMetadata: String {
        var parts: [String] = []
        if let published = published_at, let date = ISO8601DateFormatter().date(from: published) {
            parts.append(date.formatted(date: .abbreviated, time: .omitted))
        }
        if let duration = duration_seconds { parts.append("\(duration / 60) min") }
        return parts.joined(separator: " · ")
    }
}

struct DiscoverPodcastView: View {
    let item: DiscoverResultItem
    let model: DiscoverModel
    let core: HalogenCore
    @State private var preview: DiscoverPodcastData?
    @State private var error: String?
    @State private var loading = false
    @State private var subscribing = false

    private var podcast: DiscoverResultItem { preview?.podcast ?? item }

    var body: some View {
        List {
            VStack(alignment: .leading, spacing: 12) {
                Text(podcast.title).font(.title2.bold())
                if let author = podcast.author { Text(author).foregroundStyle(.secondary) }
                if let description = podcast.description, !description.isEmpty {
                    ExpandablePodcastDescription(description: description)
                }
                Button(model.isSubscribed(podcast) ? "Subscribed" : "Subscribe") {
                    subscribing = true
                    Task {
                        await model.subscribe(podcast)
                        subscribing = false
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(subscribing || loading || model.isSubscribed(podcast))
            }
            .padding(.vertical, 8)
            if loading { ProgressView("Loading episodes…") }
            if let error {
                Text(error).foregroundStyle(.red)
                Button("Retry") { Task { await load() } }
            }
            if let preview {
                Section("Episodes") {
                    ForEach(preview.episodes, id: \.id) { episode in
                        NavigationLink {
                            DiscoverEpisodeView(item: episode, model: model, core: core)
                        } label: {
                            DiscoverEpisodeRow(item: episode)
                        }
                    }
                    if preview.episodes.isEmpty { Text("No episodes in this feed.").foregroundStyle(.secondary) }
                    if preview.episodes.count >= 200 {
                        Text("Showing the first 200 episodes in this feed.").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .listStyle(.plain)
        .navigationTitle("Discover podcast")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
    }

    private func load() async {
        guard !loading, preview == nil else { return }
        loading = true
        error = nil
        defer { loading = false }
        do { preview = try await core.discoverPodcast(feedURL: item.feed_url, provider: item.provider) } catch {
            self.error = FriendlyError.message(error)
        }
    }
}

struct DiscoverEpisodeView: View {
    let item: DiscoverEpisodeItem
    let model: DiscoverModel
    let core: HalogenCore
    @State private var subscribing = false

    private var podcast: DiscoverResultItem {
        DiscoverResultItem(
            id: "parent-\(item.id)", provider: item.provider,
            title: item.podcast_title.isEmpty ? item.feed_url : item.podcast_title,
            feed_url: item.feed_url, description: nil, author: nil)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(item.title).font(.title.bold())
                NavigationLink {
                    DiscoverPodcastView(item: podcast, model: model, core: core)
                } label: {
                    Text(podcast.title)
                }
                if !item.discoverMetadata.isEmpty {
                    Text(item.discoverMetadata).font(.caption).foregroundStyle(.secondary)
                }
                Button(model.isSubscribed(podcast) ? "Subscribed" : "Subscribe to podcast") {
                    subscribing = true
                    Task {
                        await model.subscribe(podcast)
                        subscribing = false
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(subscribing || model.isSubscribed(podcast))
                Divider()
                HTMLDescription(html: item.description.isEmpty ? "No description provided." : item.description)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(20)
        }
        .navigationTitle("Discover episode")
        .navigationBarTitleDisplayMode(.inline)
    }
}
