package org.fgsec.halogen.core

import org.fgsec.halogen.features.discover.DiscoverModel
import org.fgsec.halogen.features.downloads.DownloadsModel
import org.fgsec.halogen.features.history.HistoryModel
import org.fgsec.halogen.features.latest.LatestModel
import org.fgsec.halogen.features.player.PlayerModel
import org.fgsec.halogen.features.playlists.PlaylistsModel
import org.fgsec.halogen.features.podcasts.PodcastsModel
import org.fgsec.halogen.features.queue.QueueModel

/// Per-account feature models, created together at login and torn down on
/// sign-out — one registry so cross-feature actions reach the right instance,
/// and so tab switches return to live state. An account switch rebuilds the
/// whole registry over the new namespace.
class Models(core: HalogenCore) {
    private val accountStore = core.store

    val latest = LatestModel(core)
    val podcasts = PodcastsModel(core)
    val queue = QueueModel(core)
    val playlists = PlaylistsModel(core)
    val downloads = DownloadsModel(core)
    val history = HistoryModel(core)
    val discover = DiscoverModel(core)
    val player = PlayerModel(core)
    val nav = NavModel(accountStore)
    val swipes = SwipePrefsModel(accountStore)
    val prefs = ClientPrefsModel(accountStore)
    val device = DeviceDownloads(core, core.appContext, core.account?.namespace ?: "anon", core.scope)
    val serverDownloads = ServerDownloads(core, core.scope)
    /// The optimistic playbacks overlay (cursor/played) every screen merges
    /// over its row snapshots.
    val playbacks = PlaybackOverlayModel(core, core.scope)
}
