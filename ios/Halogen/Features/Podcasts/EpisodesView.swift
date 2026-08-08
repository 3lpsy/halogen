import SwiftUI

/// One podcast's episodes, newest first — the podcast's home. The toolbar
/// menu carries podcast management (edit / download config / auto-playlists /
/// metadata / delete); Edit mode enables multi-select with bulk actions.
struct EpisodesView: View {
    let core: HalogenCore
    let podcast: PodcastData

    @Environment(\.dismiss) private var dismiss
    @State private var episodes: [EpisodeData] = []
    @State private var error: String?
    @State private var loaded = false
    @State private var hasMore = false
    @State private var loadingMore = false
    @State private var loadMoreFailed = false
    @State private var page = 0
    @State private var generation = 0
    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }
    @State private var confirmDelete = false
    @State private var manage: PodcastManageRoute?
    @State private var query = ListQuery()
    @State private var loadedQuery = false

    var body: some View {
        Group {
            if let error {
                LoadErrorView(title: "Couldn't load episodes", message: error) {
                    await load()
                }
            } else if loaded && episodes.isEmpty {
                // "No episodes yet" over an active search/filter implied an
                // unpolled feed — distinguish no-match from truly empty.
                if !query.search.isEmpty || !query.filters.isEmpty {
                    ContentUnavailableView(
                        "No matches",
                        systemImage: "line.3.horizontal.decrease.circle",
                        description: Text("No episodes match the current search or filters.")
                    )
                } else {
                    ContentUnavailableView(
                        "No episodes yet",
                        systemImage: "waveform.circle",
                        description: Text("Episodes appear after the next feed poll.")
                    )
                }
            } else {
                List(selection: $selection) {
                    // The podcast's header block (art, author, description) —
                    // the web detail's identity strip; from the already-loaded
                    // row, no extra fetch.
                    HStack(alignment: .top, spacing: 14) {
                        Artwork(url: core.podcastArtURL(podcast, small: false), size: 88)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(podcast.title).font(.headline)
                            if let author = podcast.author, !author.isEmpty {
                                Text(author).font(.subheadline).foregroundStyle(.secondary)
                            }
                            if !podcast.description.isEmpty {
                                Text(HTMLText.preview(podcast.description))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(3)
                            }
                        }
                    }
                    .listRowSeparator(.hidden)
                    .selectionDisabled()
                    ForEach(displayedEpisodes, id: \.id) { episode in
                        EpisodeRowLink(
                            episode: episode,
                            artURL: core.episodeArtURL(episode),
                            context: .browse,
                            core: core
                        )
                        .tag(episode.id)
                        .selectionDisabled(!isEditing)
                        .configuredSwipes(.podcastEpisodes, episode: episode, core: core)
                    }
                    if hasMore && !episodes.isEmpty && !selectedOnly {
                        LoadMoreRow(failed: loadMoreFailed) { await loadMore() }
                    }
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            ListControlsBar(query: $query, allowsOnDevice: !core.isEmbeddedAccount)
        }
        .navigationTitle(podcast.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                manageMenu
            }
            ToolbarItem(placement: .topBarTrailing) {
                EditButton()
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !selection.isEmpty {
                BulkActionBar(
                    selection: $selection, selectedOnly: $selectedOnly,
                    episodes: episodes, core: core)
            }
        }
        .navigationDestination(item: $manage) { route in
            PodcastManageScreen(route: route, core: core, podcast: podcast)
        }
        .confirmationDialog("Delete podcast?", isPresented: $confirmDelete) {
            Button("Delete \(podcast.title)", role: .destructive) {
                Task {
                    // Durable unsubscribe (optimistic + outbox) — works offline.
                    await core.unsubscribePodcast(id: podcast.id)
                    dismiss()
                }
            }
        } message: {
            Text("Removes the podcast, its episodes, and their server files.")
        }
        .task(id: query) { await load() }
        .refreshable { await load() }
        // Screen-local state: not covered by resyncAfterReconnect — heal a
        // stuck error state when the network returns.
        .onChange(of: core.connection.status) { _, status in
            if status == .online, error != nil {
                Task { await load() }
            }
        }
        // Leaving Edit closes multi-select for real (the bulk bar is
        // keyed on a non-empty selection).
        .onChange(of: isEditing) { _, editing in
            if !editing { selection.removeAll() }
        }
        // OnDevice chip reactivity: recompute when the device set changes
        // (see LatestView's identical hook).
        .onChange(of: core.models?.device.onDevice.map(\.id) ?? []) {
            guard isDeviceSet else { return }
            Task { await load() }
        }
    }

    private var manageMenu: some View {
        Menu {
            PodcastManageMenu(
                manage: $manage,
                confirmDelete: $confirmDelete
            )
        } label: {
            Image(systemName: "ellipsis.circle")
        }
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? episodes.filter { selection.contains($0.id) } : episodes
    }

    private var cacheKey: String { CacheKey.podcastEpisodes(podcast.id) }

    private var isDefaultQuery: Bool { query == ListQuery() }

    /// OnDevice chip (non-embedded): the list becomes this podcast's slice of
    /// the local device set (web: OnDevice branch scoped by podcast_id).
    private var isDeviceSet: Bool {
        query.filters.contains(.onDevice) && !core.isEmbeddedAccount
    }

    /// Local-first: the cached first page renders before the network answers
    /// (canonical query only), then a fresh page 0 replaces + re-snapshots.
    private func load() async {
        // Restore/persist the shared podcast-episodes list state (web:
        // use_list_view_state("podcast") — one key across all podcasts).
        if !loadedQuery {
            loadedQuery = true
            if let store = core.store,
                let saved = await store.load(ListQuery.self, key: "listquery-podcast-episodes"),
                saved != query
            {
                // Adopt and let task(id: query) refire for the restored value.
                query = saved
                return
            }
        }
        let snapshot = query
        Task { [store = core.store] in
            await store?.save(snapshot, key: "listquery-podcast-episodes")
        }
        if !query.search.isEmpty {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
        }
        if isDeviceSet {
            let device = core.models?.device
            let all = (device?.onDevice ?? []).filter {
                $0.podcast_id == podcast.id && device?.state(of: $0.id) == .downloaded
            }
            episodes = query.apply(
                to: all, status: { core.overlayStatus($0) })
            hasMore = false
            page = 0
            error = nil
            loaded = true
            return
        }
        if episodes.isEmpty, isDefaultQuery, let store = core.store,
            let cached = await store.load([EpisodeData].self, key: cacheKey)
        {
            episodes = cached
            loaded = true
        }
        generation += 1
        let mine = generation
        do {
            let first = try await core.episodes(
                podcastId: podcast.id, extra: query.queryItems, page: 0)
            guard mine == generation else { return }
            episodes = filteredForChips(first.items)
            hasMore = first.hasMore
            page = 0
            error = nil
            if isDefaultQuery {
                // Persist WITHOUT shrinking back to one page: fresh page 0
                // leads, previously cached rows keep the tail — pages the
                // user scrolled through stay renderable offline (web: every
                // fetched page is upserted into the store pool).
                let ids = Set(first.items.map(\.id))
                var snapshot = first.items
                if let prior = await core.store?.load([EpisodeData].self, key: cacheKey) {
                    snapshot += prior.filter { !ids.contains($0.id) }
                }
                await core.store?.save(snapshot, key: cacheKey)
            }
        } catch is CancellationError {
            return
        } catch let error as URLError where error.code == .cancelled {
            return
        } catch {
            guard mine == generation else { return }
            if episodes.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    /// Multi-chip facets can't ride the wire — trim each fetched page locally.
    private func filteredForChips(_ items: [EpisodeData]) -> [EpisodeData] {
        query.needsLocalChipFilter
            ? items.filter { query.matchesChips($0, status: { core.overlayStatus($0) }) }
            : items
    }

    private func loadMore() async {
        guard hasMore, !loadingMore, loaded, !isDeviceSet else { return }
        loadingMore = true
        defer { loadingMore = false }
        loadMoreFailed = false
        // Same generation guard as load(): a query change mid-fetch must drop
        // this page, not append the old query's rows (and persist them).
        let mine = generation
        do {
            let next = try await core.episodes(
                podcastId: podcast.id, extra: query.queryItems, page: page + 1)
            guard mine == generation else { return }
            page += 1
            let known = Set(episodes.map(\.id))
            episodes.append(
                contentsOf: filteredForChips(next.items.filter { !known.contains($0.id) }))
            hasMore = next.hasMore
            if isDefaultQuery {
                // Extend the offline snapshot with the appended page.
                await core.store?.save(episodes, key: cacheKey)
            }
        } catch {
            loadMoreFailed = true
        }
    }
}

/// Bulk actions over an edit-mode multi-selection — the web bulk menu's full
/// section set (bulk_menu.rs) plus the "Selected" review chip and "Select
/// all". Everything durable is offline-capable per id via the outbox.
struct BulkActionBar: View {
    @Binding var selection: Set<Int32>
    /// The "Selected" review chip: the host restricts its rows to the
    /// selection while this is on (web: MultiSelectState.selected_only).
    @Binding var selectedOnly: Bool
    let episodes: [EpisodeData]
    let core: HalogenCore
    /// The list's own context — gates "Remove from <this playlist>".
    var context: EpisodeMenuContext = .browse

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 16) {
                Text("\(selection.count) selected")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Button {
                    selectedOnly.toggle()
                } label: {
                    Text("Selected")
                        .font(.footnote.weight(.medium))
                }
                .buttonStyle(.bordered)
                .tint(selectedOnly ? Color.accentColor : .secondary)
                Button {
                    // Selects the LOADED rows only (what's fetched so far) —
                    // rows loaded afterwards are not auto-selected (web rule).
                    selection = Set(episodes.map(\.id))
                } label: {
                    Text("Select all")
                        .font(.footnote.weight(.medium))
                }
                .buttonStyle(.bordered)
                .tint(.secondary)
                Spacer()
                actionsMenu
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(.bar)
        .onDisappear { selectedOnly = false }
    }

    private var actionsMenu: some View {
        Menu {
            Section {
                Button {
                    forEachSelected { core.models?.queue.add($0) }
                } label: {
                    Label("Add to queue", systemImage: "text.badge.plus")
                }
                Button {
                    forEachSelected { core.models?.queue.remove($0) }
                } label: {
                    Label("Remove from queue", systemImage: "text.badge.minus")
                }
            }
            Section {
                Button {
                    // The shared picker dialog (RootView) resolves the target.
                    core.models?.playlists.requestPick(selectedEpisodes())
                    selection.removeAll()
                } label: {
                    Label("Add to playlist", systemImage: "music.note.list")
                }
                // Remove from the CURRENT playlist — playlist views that
                // aren't the queue (the queue section covers queue removal).
                if case .playlist(let name, let model) = context {
                    Button(role: .destructive) {
                        forEachSelected { model.remove($0) }
                    } label: {
                        Label("Remove from \(name)", systemImage: "minus.circle")
                    }
                }
            }
            serverSection
            if !core.isEmbeddedAccount {
                deviceSection
            }
        } label: {
            Image(systemName: "ellipsis.circle")
                .font(.title3)
        }
    }

    /// Server tier — embedded drops the qualifier (nothing is remote).
    private var serverSection: some View {
        let embedded = core.isEmbeddedAccount
        return Section {
            Button {
                // Remove-then-trigger in outbox order (web RedownloadOnServer).
                forEachSelected { episode in
                    Task {
                        await core.outbox?.enqueue(
                            .removeServerDownload(episodeId: episode.id))
                        core.models?.serverDownloads.download(episode)
                    }
                }
            } label: {
                Label(
                    embedded ? "Re-download" : "Re-download on server",
                    systemImage: "arrow.triangle.2.circlepath")
            }
            Button {
                // Durable trigger + progress tracking (queues offline
                // instead of silently dropping the batch).
                forEachSelected { core.models?.serverDownloads.download($0) }
            } label: {
                Label(
                    embedded ? "Download" : "Download on server",
                    systemImage: embedded ? "arrow.down.circle" : "icloud.and.arrow.down")
            }
            Button(role: .destructive) {
                forEachSelected { episode in
                    // Optimistic overlay first — rows flip immediately.
                    core.models?.serverDownloads.markRemovedLocally(episode.id)
                    Task {
                        await core.outbox?.enqueue(
                            .removeServerDownload(episodeId: episode.id))
                    }
                }
            } label: {
                Label(
                    embedded ? "Remove download" : "Remove from server",
                    systemImage: embedded ? "trash" : "icloud.slash")
            }
        }
    }

    private var deviceSection: some View {
        Section {
            Button {
                forEachSelected { episode in
                    core.models?.device.remove(episode.id)
                    core.models?.device.download(episode)
                }
            } label: {
                Label("Re-download on device", systemImage: "arrow.clockwise.circle")
            }
            Button {
                forEachSelected { core.models?.device.download($0) }
            } label: {
                Label("Download to device", systemImage: "arrow.down.to.line.circle")
            }
            Button(role: .destructive) {
                forEachSelected { core.models?.device.remove($0.id) }
            } label: {
                Label("Remove from device", systemImage: "iphone.slash")
            }
        }
    }

    private func selectedEpisodes() -> [EpisodeData] {
        episodes.filter { selection.contains($0.id) }
    }

    private func forEachSelected(_ action: (EpisodeData) -> Void) {
        for episode in episodes where selection.contains(episode.id) {
            action(episode)
        }
        selection.removeAll()
    }
}
