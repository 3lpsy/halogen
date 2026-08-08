import Foundation
import Observation

/// The Playlists tab's sort vocabulary — the web offers Custom (manual
/// position) / Name / Created (crates/ui-views playlists/controls.rs FIELDS).
enum PlaylistSortField: String, Codable, CaseIterable {
    case custom
    case name
    case created

    var label: String {
        switch self {
        case .custom: return "Custom"
        case .name: return "Name"
        case .created: return "Created"
        }
    }
}

/// Sort + search state for the Playlists tab, persisted across navigation and
/// relaunch (web: `use_list_view_state("playlists", ...)`, default Custom asc).
struct PlaylistListQuery: Codable, Equatable {
    var search: String = ""
    var field: PlaylistSortField = .custom
    var direction: OrderDirection = .asc
}

/// The Playlists tab: the user's playlists (queue included, badged), manual
/// position order. Local-first reads; create/delete are online-only (a
/// created id anchors later offline ops — same tradeoff the web makes for
/// playlist CRUD vs membership ops).
@MainActor
@Observable
final class PlaylistsModel {
    private static let queryKey = "listquery-playlists"

    private unowned let core: HalogenCore

    private(set) var playlists: [PlaylistData] = []
    private(set) var loaded = false
    private(set) var error: String?
    private var loadedQuery = false

    var query = PlaylistListQuery() {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = core.store] in await store?.save(snapshot, key: Self.queryKey) }
        }
    }
    /// Episodes awaiting a playlist choice — the native stand-in for the
    /// web's add-to-playlist picker page (swipe and bulk actions can't host
    /// a submenu). RootView presents the picker dialog while non-empty.
    var pendingPick: [EpisodeData] = []

    init(core: HalogenCore) {
        self.core = core
    }

    /// The rows the view renders: name search + Custom/Name/Created sort
    /// (web filter: apply_playlist_search_sort; Custom = `position`).
    var displayed: [PlaylistData] {
        var rows = playlists
        let q = query.search.trimmingCharacters(in: .whitespaces).lowercased()
        if !q.isEmpty {
            rows = rows.filter { $0.name.lowercased().contains(q) }
        }
        let asc = query.direction == .asc
        switch query.field {
        case .custom:
            // Pre-field cached rows decode position=nil — sort them last.
            rows.sort {
                let (a, b) = ($0.position ?? Int32.max, $1.position ?? Int32.max)
                if a != b { return asc ? a < b : a > b }
                return asc ? $0.id < $1.id : $0.id > $1.id
            }
        case .name:
            rows.sort {
                let (a, b) = ($0.name.lowercased(), $1.name.lowercased())
                if a != b { return asc ? a < b : a > b }
                return asc ? $0.id < $1.id : $0.id > $1.id
            }
        case .created:
            rows.sort {
                if $0.created_at != $1.created_at {
                    return asc
                        ? $0.created_at < $1.created_at : $0.created_at > $1.created_at
                }
                return asc ? $0.id < $1.id : $0.id > $1.id
            }
        }
        return rows
    }

    /// Manual drag-reorder is only meaningful over the unfiltered Custom-asc
    /// view, where a row's display index IS its position — and only while the
    /// visible rows are a contiguous position-prefix, or the dropped index
    /// wouldn't be a valid server position (web page.rs rules).
    var reorderable: Bool {
        guard
            query.field == .custom && query.direction == .asc
                && query.search.trimmingCharacters(in: .whitespaces).isEmpty
        else { return false }
        return displayed.enumerated().allSatisfy { idx, pl in pl.position == Int32(idx) }
    }

    /// Boot hydration (web: the worker's `hydrate_from_store`): fill the pool
    /// from the offline snapshot WITHOUT a network refresh — the row menus'
    /// playlist toggles and by-id route resolution need the pool before the
    /// Playlists tab is ever opened. `loaded` stays false.
    func seed() async {
        guard playlists.isEmpty, let store = core.store,
            let cached = await store.load([PlaylistData].self, key: CacheKey.playlists)
        else { return }
        if playlists.isEmpty { playlists = cached }
    }

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = core.store,
                let saved = await store.load(PlaylistListQuery.self, key: Self.queryKey)
            {
                query = saved
            }
        }
        if let store = core.store, playlists.isEmpty,
            let cached = await store.load([PlaylistData].self, key: CacheKey.playlists)
        {
            playlists = cached
            loaded = true
        }
        await refresh()
    }

    func refresh() async {
        // Drain first, and keep the local (optimistic) order while playlist
        // moves are still queued — server truth would snap a drag back
        // (same guard as PlaylistEpisodesModel.refresh for membership ops).
        await core.outbox?.drain()
        if let outbox = core.outbox, await outbox.hasPendingPlaylistMoves {
            loaded = true
            return
        }
        do {
            let fresh = try await core.playlistsList()
            // Membership-preservation at CACHE time (web data_ops.rs
            // cache_playlists): a playlist with queued add/remove/move ops
            // keeps its optimistic episode_ids — the server row predates the
            // ops and would flip menu checkmarks/counts back.
            var merged = fresh
            if let outbox = core.outbox {
                for (idx, playlist) in merged.enumerated() {
                    guard await outbox.hasPendingOps(playlistId: playlist.id),
                        let local = playlists.first(where: { $0.id == playlist.id }),
                        local.episode_ids != nil
                    else { continue }
                    merged[idx] = Self.rebuilt(playlist, episodeIds: local.episode_ids)
                }
            }
            playlists = merged
            error = nil
            await core.store?.save(merged, key: CacheKey.playlists)
        } catch {
            if playlists.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    func create(
        name: String, description: String?, isDefault: Bool,
        deleteServerFile: Bool, deleteClientFile: Bool
    ) async throws {
        _ = try await core.createPlaylist(
            name: name, description: description, isDefault: isDefault,
            deleteServerFile: deleteServerFile, deleteClientFile: deleteClientFile)
        await refresh()
    }

    /// Drag-reorder over the Custom-asc view: reassign positions locally so
    /// the sorted projection holds the new order, then queue the durable
    /// MovePlaylist op (web: optimistic reorder + `commands::move_playlist`).
    func move(fromOffsets: IndexSet, toOffset: Int) {
        guard reorderable, let fromIndex = fromOffsets.first else { return }
        var rows = displayed
        guard rows.indices.contains(fromIndex) else { return }
        let moved = rows[fromIndex]
        rows.move(fromOffsets: fromOffsets, toOffset: toOffset)
        guard let finalIndex = rows.firstIndex(where: { $0.id == moved.id }) else { return }
        playlists = rows.enumerated().map { idx, pl in
            Self.rebuilt(pl, position: Int32(idx))
        }
        persistSnapshot()
        Task {
            await core.outbox?.enqueue(
                .movePlaylist(playlistId: moved.id, to: Int32(finalIndex)))
        }
    }

    func delete(_ playlist: PlaylistData) async {
        playlists.removeAll { $0.id == playlist.id }
        let snapshot = playlists
        await core.store?.save(snapshot, key: CacheKey.playlists)
        // Its episode snapshot would otherwise sit on disk forever.
        await core.store?.remove(key: CacheKey.playlistEpisodes(playlist.id))
        do {
            try await core.deletePlaylist(id: playlist.id)
        } catch {
            // The refresh restores the row — without the toast that read as
            // a UI glitch, not a rejected delete.
            ToastCenter.shared.error(
                "Couldn't delete \"\(playlist.name)\" — \(FriendlyError.message(error))")
            await refresh()
        }
    }

    /// Queue a picker request (swipe "Add to playlist" / bulk add). Loads the
    /// playlist list lazily so the dialog has options on a cold start.
    func requestPick(_ episodes: [EpisodeData]) {
        guard !episodes.isEmpty else { return }
        pendingPick = episodes
        if playlists.isEmpty {
            Task { await load() }
        }
    }

    /// Deep-link fetch-through cache (web `cache_playlists`): a by-id fetched
    /// playlist joins the in-memory pool AND the offline snapshot, so the
    /// next visit — online or offline — renders instantly.
    func upsert(_ playlist: PlaylistData) {
        if let idx = playlists.firstIndex(where: { $0.id == playlist.id }) {
            playlists[idx] = playlist
        } else {
            playlists.append(playlist)
        }
        persistSnapshot()
    }

    // MARK: - optimistic mutations (offline-capable via Outbox)

    /// "Add to <playlist>" from row menus / the bulk bar: enqueue the durable
    /// op AND patch the playlist's cached episode snapshot + its episode-id
    /// membership, so the target playlist shows the episode immediately —
    /// offline included (web: add_to_playlist_locally + persist_playlist).
    func add(_ episode: EpisodeData, to playlist: PlaylistData) {
        Task {
            await core.outbox?.enqueue(
                .addToPlaylist(playlistId: playlist.id, episodeId: episode.id, position: nil))
        }
        patchMembership(playlistId: playlist.id) { ids in
            ids.contains(episode.id) ? ids : ids + [episode.id]
        }
        patchEpisodeCache(playlistId: playlist.id) { cached in
            cached.contains(where: { $0.id == episode.id }) ? cached : cached + [episode]
        }
    }

    /// Membership sync from a playlist's OWN screen: detail/queue remove/move
    /// mutate their arrays directly, bypassing add/remove here — overwrite the
    /// pool's episode_ids so menu checkmarks and counts agree with the list.
    func setMembership(playlistId: Int32, episodeIds: [Int32]) {
        guard let idx = playlists.firstIndex(where: { $0.id == playlistId }),
            playlists[idx].episode_ids != episodeIds
        else { return }
        playlists[idx] = Self.rebuilt(playlists[idx], episodeIds: episodeIds)
        persistSnapshot()
    }

    /// "Remove from <playlist>" for membership toggles outside the playlist's
    /// own screen (the row menu's checkmarked list): durable op + the same
    /// membership/episode-cache patching as `add`, mirrored (web:
    /// episode_playlists.rs diffs into remove_from_playlist ops).
    func remove(_ episode: EpisodeData, from playlist: PlaylistData) {
        Task {
            await core.outbox?.enqueue(
                .removeFromPlaylist(playlistId: playlist.id, episodeId: episode.id))
        }
        patchMembership(playlistId: playlist.id) { ids in
            ids.filter { $0 != episode.id }
        }
        patchEpisodeCache(playlistId: playlist.id) { cached in
            cached.filter { $0.id != episode.id }
        }
    }

    /// Serialized read-modify-write of a playlist's episode snapshot: the bulk
    /// picker calls add() N times, and N unchained Tasks loading the same base
    /// array before any saves is a classic lost update.
    private var cachePatchChain: Task<Void, Never>?

    private func patchEpisodeCache(
        playlistId: Int32, _ transform: @escaping ([EpisodeData]) -> [EpisodeData]
    ) {
        let store = core.store
        cachePatchChain = Task { [previous = cachePatchChain] in
            await previous?.value
            let key = CacheKey.playlistEpisodes(playlistId)
            let cached = await store?.load([EpisodeData].self, key: key) ?? []
            await store?.save(transform(cached), key: key)
        }
    }

    /// Offline rename: patch screen + snapshot (the queued UpdatePlaylist op
    /// syncs the server; online renames go direct so forms can show errors).
    func renameLocally(id: Int32, name: String) {
        patch(id: id) { Self.rebuilt($0, name: name) }
    }

    /// Offline full-edit: apply the web form's field set locally (the queued
    /// UpdatePlaylist op syncs the server on drain).
    func updateLocally(
        id: Int32, name: String, description: String?,
        isDefault: Bool, deleteServerFile: Bool, deleteClientFile: Bool
    ) {
        patch(id: id) {
            Self.rebuilt(
                $0, name: name, description: .some(description),
                deleteServerFile: deleteServerFile, deleteClientFile: deleteClientFile)
        }
        if isDefault {
            markDefaultLocally(id: id)
        }
    }

    /// Offline make-queue: flip `is_default` locally so the badge moves (the
    /// server demotes the old default in the same transaction on drain).
    func markDefaultLocally(id: Int32) {
        var changed = false
        for idx in playlists.indices where playlists[idx].is_default != (playlists[idx].id == id) {
            playlists[idx] = Self.rebuilt(playlists[idx], isDefault: playlists[idx].id == id)
            changed = true
        }
        guard changed else { return }
        persistSnapshot()
    }

    private func patch(id: Int32, _ transform: (PlaylistData) -> PlaylistData) {
        guard let idx = playlists.firstIndex(where: { $0.id == id }) else { return }
        playlists[idx] = transform(playlists[idx])
        persistSnapshot()
    }

    private func patchMembership(playlistId: Int32, _ transform: ([Int32]) -> [Int32]) {
        guard let idx = playlists.firstIndex(where: { $0.id == playlistId }),
            let ids = playlists[idx].episode_ids
        else { return }
        let newIds = transform(ids)
        guard newIds != ids else { return }
        playlists[idx] = Self.rebuilt(playlists[idx], episodeIds: newIds)
        persistSnapshot()
    }

    private func persistSnapshot() {
        let snapshot = playlists
        Task { [store = core.store] in await store?.save(snapshot, key: CacheKey.playlists) }
    }

    /// Generated DTOs are immutable (let fields) — rebuild with changes.
    /// `description` is double-optional: nil = keep, .some(x) = set to x.
    private static func rebuilt(
        _ p: PlaylistData,
        name: String? = nil,
        description: String?? = nil,
        isDefault: Bool? = nil,
        position: Int32? = nil,
        deleteServerFile: Bool? = nil,
        deleteClientFile: Bool? = nil,
        episodeIds: [Int32]? = nil
    ) -> PlaylistData {
        PlaylistData(
            id: p.id, name: name ?? p.name,
            description: description ?? p.description,
            is_default: isDefault ?? p.is_default,
            position: position ?? p.position,
            on_remove_delete_file_server: deleteServerFile ?? p.on_remove_delete_file_server,
            on_remove_delete_file_client: deleteClientFile ?? p.on_remove_delete_file_client,
            created_at: p.created_at, updated_at: p.updated_at,
            episode_ids: episodeIds ?? p.episode_ids,
            episode_playlist: p.episode_playlist)
    }
}
