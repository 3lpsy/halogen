package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.storage.LocalStore
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// Configurable episode swipe actions — the full web vocabulary (webui/listview
/// `SwipeAction::ALL`), one per edge per page. Toggles resolve against the row's live
/// state; embedded accounts remap device-download actions to their server counterparts.
@Serializable
enum class SwipeAction(val token: String) {
    @SerialName("none") None("none"),
    @SerialName("play") Play("play"),
    @SerialName("stream") Stream("stream"),
    @SerialName("toggle_played") TogglePlayed("toggle_played"),
    @SerialName("mark_played") MarkPlayed("mark_played"),
    @SerialName("mark_unplayed") MarkUnplayed("mark_unplayed"),
    @SerialName("reset_progress") ResetProgress("reset_progress"),
    @SerialName("add_to_queue") AddToQueue("add_to_queue"),
    @SerialName("remove_from_queue") RemoveFromQueue("remove_from_queue"),
    @SerialName("toggle_queue") ToggleQueue("toggle_queue"),
    /// Remove from *this list's* playlist (queue or the viewed playlist).
    @SerialName("remove_from_list") RemoveFromList("remove_from_list"),
    @SerialName("add_to_playlist") AddToPlaylist("add_to_playlist"),
    /// Keeps the pre-parity "download" token so saved prefs still decode.
    @SerialName("download") DownloadToDevice("download"),
    @SerialName("redownload_device") RedownloadDevice("redownload_device"),
    @SerialName("remove_download") RemoveDownload("remove_download"),
    @SerialName("toggle_download") ToggleDownload("toggle_download"),
    @SerialName("download_on_server") DownloadOnServer("download_on_server"),
    @SerialName("remove_from_server") RemoveFromServer("remove_from_server"),
    @SerialName("toggle_server_download") ToggleServerDownload("toggle_server_download");

    val label: String
        get() = when (this) {
            None -> "None"
            Play -> "Play"
            Stream -> "Stream"
            TogglePlayed -> "Toggle played"
            MarkPlayed -> "Mark played"
            MarkUnplayed -> "Mark unplayed"
            ResetProgress -> "Reset progress"
            AddToQueue -> "Add to queue"
            RemoveFromQueue -> "Remove from queue"
            ToggleQueue -> "Toggle queue"
            RemoveFromList -> "Remove from list"
            AddToPlaylist -> "Add to playlist"
            DownloadToDevice -> "Download"
            RedownloadDevice -> "Re-download"
            RemoveDownload -> "Remove download"
            ToggleDownload -> "Toggle download"
            DownloadOnServer -> "Download on server"
            RemoveFromServer -> "Remove from server"
            ToggleServerDownload -> "Toggle server download"
        }

    /// SF Symbol name — components/Icons.kt maps it to a Material icon.
    val systemImage: String
        get() = when (this) {
            None -> "slash.circle"
            Play -> "play.fill"
            Stream -> "dot.radiowaves.left.and.right"
            TogglePlayed -> "checkmark.circle.badge.questionmark"
            MarkPlayed -> "checkmark.circle"
            MarkUnplayed -> "circle"
            ResetProgress -> "arrow.counterclockwise"
            AddToQueue -> "text.badge.plus"
            RemoveFromQueue -> "text.badge.minus"
            ToggleQueue -> "text.badge.checkmark"
            RemoveFromList -> "minus.circle"
            AddToPlaylist -> "music.note.list"
            DownloadToDevice -> "arrow.down.to.line.circle"
            RedownloadDevice -> "arrow.triangle.2.circlepath"
            RemoveDownload -> "iphone.slash"
            ToggleDownload -> "arrow.down.circle.dotted"
            DownloadOnServer -> "icloud.and.arrow.down"
            RemoveFromServer -> "icloud.slash"
            ToggleServerDownload -> "icloud.circle"
        }

    /// Destructive/removal actions get the red tint; additive ones blue-ish.
    /// Semantic name only — components/Theme.kt maps it to a Color.
    val tint: SwipeTint
        get() = when (this) {
            None -> SwipeTint.Gray
            Play, Stream -> SwipeTint.Indigo
            TogglePlayed, MarkPlayed, MarkUnplayed, ResetProgress -> SwipeTint.Green
            AddToQueue, ToggleQueue -> SwipeTint.Blue
            RemoveFromQueue, RemoveFromList -> SwipeTint.Red
            AddToPlaylist -> SwipeTint.Purple
            DownloadToDevice, RedownloadDevice, ToggleDownload,
            DownloadOnServer, ToggleServerDownload -> SwipeTint.Orange
            RemoveDownload, RemoveFromServer -> SwipeTint.Red
        }
}

/// The iOS system-color vocabulary the swipe tints use.
enum class SwipeTint { Gray, Indigo, Green, Blue, Red, Purple, Orange }

/// The pages whose swipes are configurable — all six of the web's
/// (webui/config swipe.rs `SwipePage::ALL`).
@Serializable
enum class SwipePage(val token: String) {
    @SerialName("latest") Latest("latest"),
    @SerialName("queue") Queue("queue"),
    @SerialName("podcast_episodes") PodcastEpisodes("podcast_episodes"),
    @SerialName("downloads") Downloads("downloads"),
    @SerialName("history") History("history"),
    @SerialName("playlist") Playlist("playlist");

    val label: String
        get() = when (this) {
            Latest -> "Latest"
            Queue -> "Queue"
            PodcastEpisodes -> "Podcast episodes"
            Downloads -> "Downloads"
            History -> "History"
            Playlist -> "Playlist"
        }

    /// Actions offered for this page, in menu order — `RemoveFromList` only
    /// where the list IS a playlist (web: `SwipePage::allowed_actions`).
    val allowedActions: List<SwipeAction>
        get() = SwipeAction.entries.filter { action ->
            action != SwipeAction.RemoveFromList || this == Queue || this == Playlist
        }
}

@Serializable
data class SwipePrefs(val pages: Map<String, PagePrefs>) {
    @Serializable
    data class PagePrefs(val leading: SwipeAction, val trailing: SwipeAction)

    /// Unset pages (saved prefs predating a page) fall back to the web
    /// default, not to a dead swipe.
    fun page(page: SwipePage): PagePrefs =
        pages[page.token]
            ?: default.pages[page.token]
            ?: PagePrefs(SwipeAction.None, SwipeAction.None)

    companion object {
        /// The web defaults (webui/config swipe.rs): `leading` here is the
        /// web's `left` (fires on a swipe-RIGHT gesture), `trailing` its `right`.
        val default = SwipePrefs(
            pages = mapOf(
                SwipePage.Latest.token to
                    PagePrefs(SwipeAction.AddToQueue, SwipeAction.DownloadToDevice),
                SwipePage.Queue.token to
                    PagePrefs(SwipeAction.RemoveFromQueue, SwipeAction.DownloadToDevice),
                SwipePage.PodcastEpisodes.token to
                    PagePrefs(SwipeAction.AddToPlaylist, SwipeAction.DownloadToDevice),
                SwipePage.Downloads.token to
                    PagePrefs(SwipeAction.AddToPlaylist, SwipeAction.RedownloadDevice),
                SwipePage.History.token to
                    PagePrefs(SwipeAction.AddToPlaylist, SwipeAction.DownloadToDevice),
                SwipePage.Playlist.token to
                    PagePrefs(SwipeAction.RemoveFromList, SwipeAction.DownloadToDevice),
            )
        )
    }
}

/// Reactive holder + per-account persistence.
class SwipePrefsModel(
    private val store: LocalStore?,
    private val scope: CoroutineScope = MainScope(),
) {
    var prefs: SwipePrefs by mutableStateOf(SwipePrefs.default)
        private set

    suspend fun load() {
        val saved = store?.load<SwipePrefs>(KEY) ?: return
        prefs = saved
    }

    fun update(page: SwipePage, leading: SwipeAction, trailing: SwipeAction) {
        prefs = prefs.copy(
            pages = prefs.pages + (page.token to SwipePrefs.PagePrefs(leading, trailing)))
        val snapshot = prefs
        scope.launch { store?.save(snapshot, KEY) }
    }

    private companion object {
        const val KEY = "swipe-prefs"
    }
}

/// The action→primitive mapping — one place, mirroring the web's
/// `perform_episode_action` (webui/episode-list action.rs), embedded
/// device→server remaps included. The Compose swipe container
/// (components/SwipeActions.kt) and row menus both call through here.
fun performSwipeAction(
    action: SwipeAction,
    episode: EpisodeData,
    core: HalogenCore,
    context: EpisodeMenuContext = EpisodeMenuContext.Browse,
) {
    val models = core.models ?: return
    val embedded = core.isEmbeddedAccount
    when (action) {
        SwipeAction.None -> {}
        SwipeAction.Play ->
            models.player.play(episode, context.playbackContext)
        SwipeAction.Stream ->
            models.player.stream(episode, context.playbackContext)
        SwipeAction.TogglePlayed -> {
            val status = models.playbacks.status(episode)
            models.playbacks.markPlayed(episode, status != PlaybackStatus.Finished)
        }
        SwipeAction.MarkPlayed -> models.playbacks.markPlayed(episode, true)
        SwipeAction.MarkUnplayed -> models.playbacks.markPlayed(episode, false)
        SwipeAction.ResetProgress -> models.playbacks.setCursor(episode, 0uL)
        SwipeAction.AddToQueue -> models.queue.add(episode)
        SwipeAction.RemoveFromQueue -> models.queue.remove(episode)
        SwipeAction.ToggleQueue ->
            if (models.queue.contains(episode)) models.queue.remove(episode)
            else models.queue.add(episode)
        SwipeAction.RemoveFromList -> when (context) {
            is EpisodeMenuContext.Queue -> models.queue.remove(episode)
            is EpisodeMenuContext.Playlist -> context.model.remove(episode)
            is EpisodeMenuContext.Browse -> {} // not offered on browse pages (allowedActions)
        }
        SwipeAction.AddToPlaylist -> models.playlists.requestPick(listOf(episode))
        SwipeAction.DownloadToDevice ->
            if (embedded) models.serverDownloads.download(episode)
            else models.device.download(episode)
        SwipeAction.RedownloadDevice -> when {
            embedded -> redownloadServer(core, episode)
            !core.isOffline -> {
                // Web rule: never destroy a local copy you can't re-pull.
                models.device.remove(episode.id)
                models.device.download(episode)
            }
            // The refusal is right; the silence wasn't — the swipe just
            // animated closed with no effect.
            else -> ToastCenter.error("You're offline — re-download needs a connection.")
        }
        SwipeAction.RemoveDownload ->
            if (embedded) removeServer(core, episode)
            else models.device.remove(episode.id)
        SwipeAction.ToggleDownload -> when {
            embedded -> toggleServer(core, episode)
            models.device.stateOf(episode.id) == DeviceDownloads.State.Downloaded ->
                models.device.remove(episode.id)
            else -> models.device.download(episode)
        }
        SwipeAction.DownloadOnServer -> models.serverDownloads.download(episode)
        SwipeAction.RemoveFromServer -> removeServer(core, episode)
        SwipeAction.ToggleServerDownload -> toggleServer(core, episode)
    }
}

private fun removeServer(core: HalogenCore, episode: EpisodeData) {
    core.enqueueMutation(OutboxOp.Kind.RemoveServerDownload(episode.id)) {
        core.models?.serverDownloads?.markRemovedLocally(episode.id)
    }
}

/// Remove-then-trigger, in outbox order (web: RedownloadOnServer).
private fun redownloadServer(core: HalogenCore, episode: EpisodeData) {
    core.scope.launch {
        if (!core.ensureQueuedBatch(listOf(OutboxOp.Kind.RemoveServerDownload(episode.id), OutboxOp.Kind.TriggerDownload(episode.id)))) return@launch
        core.models?.serverDownloads?.watch(episode.id)
    }
}

private fun toggleServer(core: HalogenCore, episode: EpisodeData) {
    if (core.models?.serverDownloads?.isDownloaded(episode) == true) {
        removeServer(core, episode)
    } else {
        core.models?.serverDownloads?.download(episode)
    }
}
