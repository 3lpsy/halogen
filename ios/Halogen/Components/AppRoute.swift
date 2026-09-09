import SwiftUI

/// Typed navigation targets shared by every tab's NavigationStack — one
/// vocabulary so any list can push any detail without per-stack destination
/// collisions (the Swift analog of the web's Route enum, entity slice).
enum AppRoute: Hashable {
    case podcast(Int32)
    case episode(Int32)
    case playlist(Int32)
    /// The episode metadata (raw key/value) screen — a route because it's
    /// pushed from a Menu, and menu items can't drive NavigationLinks.
    case episodeMetadata(Int32)
}

/// Per-tab programmatic navigation: rows/menu items push routes here instead of
/// embedding NavigationLinks — a List row tap activates EVERY automatic-style
/// link at once, and menu items can't drive NavigationLinks at all.
@MainActor
@Observable
final class Navigator {
    var path = NavigationPath()

    func push(_ route: AppRoute) {
        path.append(route)
    }
}

extension View {
    /// Register the shared destinations on a NavigationStack's root.
    func appDestinations(core: HalogenCore, models: Models) -> some View {
        navigationDestination(for: AppRoute.self) { route in
            switch route {
            case .podcast(let id):
                PodcastScreen(id: id, core: core, models: models)
            case .episode(let id):
                EpisodeDetailView(core: core, episodeId: id)
            case .playlist(let id):
                PlaylistScreen(id: id, core: core, models: models)
            case .episodeMetadata(let id):
                JSONKeyValueView(
                    title: "Metadata", core: core, path: "episodes/\(id)",
                    cacheKey: CacheKey.metadata("episodes/\(id)"))
            }
        }
    }
}

/// Resolves a podcast id local-first (web podcast_detail.rs): pool → offline
/// snapshot → network, upserting the fetched row so the next visit is instant/offline.
private struct PodcastScreen: View {
    let id: Int32
    let core: HalogenCore
    let models: Models

    @State private var resolved: PodcastData?
    @State private var failure: String?

    var body: some View {
        Group {
            if let podcast = models.podcasts.podcasts.first(where: { $0.id == id }) ?? resolved {
                EpisodesView(core: core, podcast: podcast)
            } else if let failure {
                ContentUnavailableView {
                    Label(
                        core.isOffline ? "Podcast not cached" : "Couldn't load podcast",
                        systemImage: core.isOffline ? "wifi.slash" : "waveform.circle")
                } description: {
                    Text(failure).font(.footnote.monospaced())
                } actions: {
                    Button("Try again") { self.failure = nil }
                        .buttonStyle(.borderedProminent)
                }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task { await resolve() }
            }
        }
        // An offline miss recovers on its own once connectivity returns
        // (web: the deep-link hook re-runs on a later publish).
        .onChange(of: core.isOffline) { _, offline in
            if !offline, failure != nil { failure = nil }
        }
    }

    private func resolve() async {
        // Offline snapshot (everything the library list ever fetched).
        if let cached = await core.store?.load([PodcastData].self, key: CacheKey.podcasts),
            let hit = cached.first(where: { $0.id == id })
        {
            resolved = hit
            // Stale-while-revalidate: freshen the row in the background.
            Task {
                if let fresh = try? await core.podcast(id: id) {
                    models.podcasts.upsert(fresh)
                    resolved = fresh
                }
            }
            return
        }
        // Genuine miss: fetch-through, then cache for next time (web:
        // get_podcast + cache_podcasts on the deep-link path).
        do {
            let fresh = try await core.podcast(id: id)
            models.podcasts.upsert(fresh)
            resolved = fresh
        } catch {
            failure =
                core.isOffline
                ? "This podcast hasn't been cached on this device yet."
                : FriendlyError.message(error)
        }
    }
}

/// Same local-first resolution for playlists: pool → snapshot → network,
/// with the fetched row upserted for offline re-visits.
private struct PlaylistScreen: View {
    let id: Int32
    let core: HalogenCore
    let models: Models

    @State private var resolved: PlaylistData?
    @State private var failure: String?

    var body: some View {
        Group {
            if let playlist = models.playlists.playlists.first(where: { $0.id == id }) ?? resolved {
                PlaylistDetailView(core: core, playlist: playlist)
            } else if let failure {
                ContentUnavailableView {
                    Label(
                        core.isOffline ? "Playlist not cached" : "Couldn't load playlist",
                        systemImage: core.isOffline ? "wifi.slash" : "music.note.list")
                } description: {
                    Text(failure).font(.footnote.monospaced())
                } actions: {
                    Button("Try again") { self.failure = nil }
                        .buttonStyle(.borderedProminent)
                }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task { await resolve() }
            }
        }
        .onChange(of: core.isOffline) { _, offline in
            if !offline, failure != nil { failure = nil }
        }
    }

    private func resolve() async {
        if let cached = await core.store?.load([PlaylistData].self, key: CacheKey.playlists),
            let hit = cached.first(where: { $0.id == id })
        {
            resolved = hit
            Task {
                if let fresh = try? await core.playlist(id: id) {
                    models.playlists.upsert(fresh)
                    resolved = fresh
                }
            }
            return
        }
        do {
            let fresh = try await core.playlist(id: id)
            models.playlists.upsert(fresh)
            resolved = fresh
        } catch {
            failure =
                core.isOffline
                ? "This playlist hasn't been cached on this device yet."
                : FriendlyError.message(error)
        }
    }
}
