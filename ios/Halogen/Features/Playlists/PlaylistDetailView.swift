import SwiftUI

/// One playlist's episodes in position order — same offline-capable
/// remove/reorder as the queue (they share the outbox op vocabulary and the
/// per-playlist episode cache key).
struct PlaylistDetailView: View {
    let core: HalogenCore
    let playlist: PlaylistData

    @State private var model: PlaylistEpisodesModel
    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }

    init(core: HalogenCore, playlist: PlaylistData) {
        self.core = core
        self.playlist = playlist
        _model = State(initialValue: PlaylistEpisodesModel(core: core, playlistId: playlist.id))
    }

    var body: some View {
        Group {
            if let error = model.error {
                LoadErrorView(title: "Couldn't load playlist", message: error) {
                    await model.refresh()
                }
            } else if model.loaded && model.episodes.isEmpty {
                ContentUnavailableView(
                    "Empty playlist",
                    systemImage: "music.note.list",
                    description: Text("Add episodes from any list via their context menu.")
                )
            } else if model.loaded && model.displayed.isEmpty {
                ContentUnavailableView(
                    "No matches",
                    systemImage: "line.3.horizontal.decrease.circle",
                    description: Text("No episodes match the current search or filters.")
                )
            } else {
                List(selection: $selection) {
                    ForEach(displayedEpisodes, id: \.id) { episode in
                        EpisodeRowLink(
                            episode: episode,
                            artURL: core.episodeArtURL(episode),
                            subtitle: episode.podcast?.title,
                            context: .playlist(name: playlist.name, model: model),
                            core: core
                        )
                        .tag(episode.id)
                        .selectionDisabled(!isEditing)
                        .configuredSwipes(
                            .playlist, episode: episode, core: core,
                            context: .playlist(name: playlist.name, model: model)
                        )
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                model.remove(episode)
                            } label: {
                                Label("Remove", systemImage: "minus.circle")
                            }
                        }
                    }
                    .onMove { from, to in
                        // Reorder only over the raw position order with no
                        // search/filter (queue rule; web parity): a subset's
                        // indices don't map onto the full array — a drag would
                        // silently reorder hidden rows and sync that corruption.
                        guard model.reorderable, !selectedOnly else { return }
                        model.move(fromOffsets: from, toOffset: to)
                    }
                    .moveDisabled(!model.reorderable || selectedOnly)
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            ListControlsBar(
                query: $model.query, allowsPosition: true,
                allowsOnDevice: !core.isEmbeddedAccount)
        }
        .navigationTitle(playlist.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                EditButton()
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !selection.isEmpty {
                BulkActionBar(
                    selection: $selection, selectedOnly: $selectedOnly,
                    episodes: model.displayed, core: core,
                    context: .playlist(name: playlist.name, model: model))
            }
        }
        .task { await model.load() }
        .refreshable { await model.refresh() }
        // Screen-local model: not covered by resyncAfterReconnect — heal a
        // stuck error state when the network returns.
        .onChange(of: core.connection.status) { _, status in
            if status == .online, model.error != nil {
                Task { await model.refresh() }
            }
        }
        // Leaving Edit closes multi-select for real (the bulk bar is
        // keyed on a non-empty selection).
        .onChange(of: isEditing) { _, editing in
            if !editing { selection.removeAll() }
        }
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? model.displayed.filter { selection.contains($0.id) } : model.displayed
    }
}

/// Episodes-of-a-playlist state, shared semantics with the queue (which is
/// just the default playlist): cache-first render, optimistic outbox ops.
@MainActor
@Observable
final class PlaylistEpisodesModel {
    /// One shared query across every playlist detail (web: the "playlist"
    /// list-view key is shared, default Custom/position order).
    private static let queryKey = "listquery-playlist"

    private unowned let core: HalogenCore
    /// Exposed so play actions can carry this list as the play context.
    let playlistId: Int32

    private(set) var episodes: [EpisodeData] = []
    private(set) var loaded = false
    private(set) var error: String?
    private var loadedQuery = false
    /// Bumped synchronously by every optimistic mutation; a refresh discards
    /// its response if this moved mid-fetch (see QueueModel.mutationEpoch).
    private var mutationEpoch = 0

    /// Search + chips + order, applied locally over the position-ordered
    /// membership (same as the queue — this list IS a playlist).
    var query = ListQuery(orderField: .position, direction: .asc) {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = core.store] in await store?.save(snapshot, key: Self.queryKey) }
        }
    }

    /// The rows the view renders (query over the raw membership).
    var displayed: [EpisodeData] {
        query.apply(
            to: episodes,
            isOnDevice: { [weak core] id in core?.models?.device.state(of: id) == .downloaded },
            status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed }
        )
    }

    /// Reordering is only meaningful over the raw position order with no
    /// search/filter applied (queue rule).
    var reorderable: Bool {
        query == ListQuery(orderField: .position, direction: .asc)
    }

    init(core: HalogenCore, playlistId: Int32) {
        self.core = core
        self.playlistId = playlistId
    }

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = core.store,
                let saved = await store.load(ListQuery.self, key: Self.queryKey)
            {
                query = saved
            }
        }
        if let store = core.store, episodes.isEmpty,
            let cached = await store.load(
                [EpisodeData].self, key: CacheKey.playlistEpisodes(playlistId))
        {
            episodes = cached
            loaded = true
        }
        await refresh()
    }

    func refresh() async {
        // Drain BEFORE pulling, and keep the local list when membership ops
        // are still queued — same rationale as QueueModel.refresh (the queue
        // is just the default playlist).
        await core.outbox?.drain()
        if let outbox = core.outbox, await outbox.hasPendingOps(playlistId: playlistId) {
            loaded = true
            return
        }
        do {
            let epoch = mutationEpoch
            let fresh = try await core.playlistEpisodes(playlistId: playlistId)
            // Guard AGAIN at cache time: a mutation that landed while the
            // fetch was in flight outranks the stale server membership.
            if epoch != mutationEpoch { loaded = true; return }
            if let outbox = core.outbox, await outbox.hasPendingOps(playlistId: playlistId) {
                loaded = true
                return
            }
            episodes = fresh
            error = nil
            await core.store?.save(fresh, key: CacheKey.playlistEpisodes(playlistId))
        } catch {
            if episodes.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    func remove(_ episode: EpisodeData) {
        mutationEpoch += 1
        episodes.removeAll { $0.id == episode.id }
        core.models?.playlists.setMembership(playlistId: playlistId, episodeIds: episodes.map(\.id))
        persistSnapshot()
        Task {
            await core.outbox?.enqueue(
                .removeFromPlaylist(playlistId: playlistId, episodeId: episode.id))
        }
    }

    func move(fromOffsets: IndexSet, toOffset: Int) {
        guard let fromIndex = fromOffsets.first,
            episodes.indices.contains(fromIndex)
        else { return }
        mutationEpoch += 1
        let moved = episodes[fromIndex]
        episodes.move(fromOffsets: fromOffsets, toOffset: toOffset)
        guard let finalIndex = episodes.firstIndex(where: { $0.id == moved.id }) else { return }
        core.models?.playlists.setMembership(playlistId: playlistId, episodeIds: episodes.map(\.id))
        persistSnapshot()
        Task {
            await core.outbox?.enqueue(
                .moveInPlaylist(playlistId: playlistId, episodeId: moved.id, to: Int32(finalIndex)))
        }
    }

    private func persistSnapshot() {
        let snapshot = episodes
        Task { [store = core.store, playlistId] in
            await store?.save(snapshot, key: CacheKey.playlistEpisodes(playlistId))
        }
    }
}
