import Foundation

/// Per-account feature models, created at login and torn down on sign-out —
/// one registry so cross-feature actions reach the right instance and tab
/// switches return to live state. An account switch rebuilds it wholesale.
@MainActor
final class Models {
    let latest: LatestModel
    let podcasts: PodcastsModel
    let queue: QueueModel
    let playlists: PlaylistsModel
    let downloads: DownloadsModel
    let history: HistoryModel
    let discover: DiscoverModel
    let player: PlayerModel
    let nav: NavModel
    let swipes: SwipePrefsModel
    let prefs: ClientPrefsModel
    let device: DeviceDownloads
    let serverDownloads: ServerDownloads
    /// The optimistic playbacks overlay (cursor/played) every screen merges
    /// over its row snapshots — the web's shared playback state.
    let playbacks: PlaybackOverlayModel

    init(core: HalogenCore) {
        latest = LatestModel(core: core)
        podcasts = PodcastsModel(core: core)
        queue = QueueModel(core: core)
        playlists = PlaylistsModel(core: core)
        downloads = DownloadsModel(core: core)
        history = HistoryModel(core: core)
        discover = DiscoverModel(core: core)
        player = PlayerModel(core: core)
        nav = NavModel(store: core.store)
        swipes = SwipePrefsModel(store: core.store)
        prefs = ClientPrefsModel(store: core.store)
        device = DeviceDownloads(core: core, namespace: core.account?.namespace ?? "anon")
        serverDownloads = ServerDownloads(core: core)
        playbacks = PlaybackOverlayModel(core: core)
    }
}
