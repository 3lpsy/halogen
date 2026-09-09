package org.fgsec.halogen.features.episode

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlin.coroutines.cancellation.CancellationException
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// One episode's detail state: cached per id (offline re-open), refreshed
/// with Podcast/Playback/Chapters includes; played + download actions.
class EpisodeDetailModel(
    private val core: HalogenCore,
    private val episodeId: Int,
) {
    private val accountStore = core.store

    var episode: EpisodeData? by mutableStateOf(null)
        private set
    var error: String? by mutableStateOf(null)
        private set

    suspend fun load() {
        if (episode == null) {
            accountStore?.load<EpisodeData>(CacheKey.episode(episodeId))?.let { episode = it }
        }
        refresh()
    }

    suspend fun refresh() {
        try {
            val fresh = core.episodeDetail(episodeId)
            episode = fresh
            error = null
            accountStore?.save(fresh, CacheKey.episode(episodeId))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn(
                "episode-detail $episodeId: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (episode == null) error = FriendlyError.message(e)
        }
    }

    /// Toggle finished ↔ unplayed. Optimistic + outbox (works offline): the
    /// shared playbacks overlay does the enqueue and flips every other screen
    /// in lock-step; this model also patches its own cached copy.
    suspend fun togglePlayed() {
        var current = episode ?: return
        val status = core.models?.playbacks?.status(current)
            ?: current.playback_status ?: PlaybackStatus.Unplayed
        val nowPlayed = status != PlaybackStatus.Finished
        core.models?.playbacks?.markPlayed(current, nowPlayed)
        current = current.copy(
            playback_status =
                if (nowPlayed) PlaybackStatus.Finished else PlaybackStatus.Unplayed)
        episode = current
        accountStore?.save(current, CacheKey.episode(episodeId))
    }
}
