import SwiftUI

/// Which playlists this podcast auto-adds new episodes to (web
/// `/podcasts/:id/auto-playlists`). PUT replaces the whole set, so editing is
/// GATED until the current set has loaded — an unconfirmed (empty) selection
/// would wipe the server's real selections.
struct AutoPlaylistsView: View {
    let core: HalogenCore
    let podcast: PodcastData

    @State private var selected: Set<Int32> = []
    /// Insert-position override for auto-added episodes: true = start,
    /// false = end, nil = server default (stamped on every link).
    @State private var addToStart: Bool?
    @State private var loaded = false
    @State private var error: String?
    /// The user touched the set this visit — a late network answer must not
    /// snap their toggles back (the queued outbox op is the newer truth).
    @State private var edited = false

    var body: some View {
        List {
            if !loaded && error == nil {
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Loading auto-playlists…").foregroundStyle(.secondary)
                }
            }
            Section {
                ForEach(core.models?.playlists.playlists ?? [], id: \.id) { playlist in
                    Button {
                        toggle(playlist.id)
                    } label: {
                        HStack {
                            Text(playlist.name).foregroundStyle(.primary)
                            if playlist.is_default {
                                Text("Queue").font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if selected.contains(playlist.id) {
                                Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                            }
                        }
                    }
                    .disabled(!loaded)
                }
            } footer: {
                Text("New episodes from \(podcast.title) are added to the checked playlists on each poll.")
            }
            Section {
                // A manual binding so only USER changes save — the initial
                // seed from load() must not fire a redundant PUT.
                Picker(
                    "New episodes are added to",
                    selection: Binding(
                        get: { addToStart },
                        set: { newValue in
                            addToStart = newValue
                            // The override rides every link, so a position
                            // change is a save of the same whole set.
                            if loaded {
                                edited = true
                                save()
                            }
                        }
                    )
                ) {
                    Text("Server default").tag(Bool?.none)
                    Text("Start of playlist").tag(Bool?.some(true))
                    Text("End of playlist").tag(Bool?.some(false))
                }
                .disabled(!loaded)
            }
            if let error {
                HStack {
                    Text(error).font(.footnote).foregroundStyle(.red)
                    Spacer()
                    if !loaded {
                        Button("Retry") {
                            Task { await load() }
                        }
                        .font(.footnote)
                    }
                }
            }
        }
        .navigationTitle("Auto-playlists")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            await core.models?.playlists.load()
            await seedFromCache()
            await load()
        }
    }

    /// Local-first seed (web podcast_auto_playlists.rs: the cached set makes
    /// the screen readable AND editable offline; the PUT-wipe hazard the
    /// `loaded` gate protects against doesn't apply to a confirmed snapshot).
    private func seedFromCache() async {
        guard !loaded, let store = core.store,
            let cached = await store.load(
                AutoPlaylistsSnapshot.self, key: CacheKey.autoPlaylists(podcast.id))
        else { return }
        selected = Set(cached.playlistIds)
        addToStart = cached.addToStart
        loaded = true
    }

    private func load() async {
        error = nil
        do {
            let links = try await core.autoPlaylists(podcastId: podcast.id)
            if !edited {
                selected = Set(links.map(\.playlist_id))
                // Every link carries the same per-podcast override (the set
                // endpoint stamps it uniformly) — the first row speaks for
                // the set.
                addToStart = links.first?.add_to_start
            }
            loaded = true
            error = nil
            await core.store?.save(
                AutoPlaylistsSnapshot(
                    playlistIds: links.map(\.playlist_id),
                    addToStart: links.first?.add_to_start),
                key: CacheKey.autoPlaylists(podcast.id))
        } catch {
            // A cache-seeded screen stays editable; only a cold miss blocks.
            if !loaded { self.error = FriendlyError.message(error) }
        }
    }

    private func toggle(_ id: Int32) {
        // Never edit an unconfirmed set — see the type doc.
        guard loaded else { return }
        edited = true
        if selected.contains(id) {
            selected.remove(id)
        } else {
            selected.insert(id)
        }
        save()
    }

    /// Durable replace-the-set op (web: SetPodcastAutoPlaylists) — the
    /// checkmarks above are the optimistic state; queues offline, and the
    /// FIFO outbox keeps rapid edits in order.
    private func save() {
        let ids = Array(selected)
        let position = addToStart
        Task {
            await core.outbox?.enqueue(
                .setAutoPlaylists(
                    podcastId: podcast.id, playlistIds: ids, addToStart: position))
            // The snapshot tracks the queued truth, so an offline re-open
            // shows what will land on drain.
            await core.store?.save(
                AutoPlaylistsSnapshot(playlistIds: ids, addToStart: position),
                key: CacheKey.autoPlaylists(podcast.id))
            error = nil
        }
    }
}

/// The cached auto-playlist selection (ids + insert-position override).
private struct AutoPlaylistsSnapshot: Codable {
    let playlistIds: [Int32]
    let addToStart: Bool?
}
