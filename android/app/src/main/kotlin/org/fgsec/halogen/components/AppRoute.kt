package org.fgsec.halogen.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.compose.composable
import androidx.navigation.toRoute
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.Models
import org.fgsec.halogen.features.episode.EpisodeDetailView
import org.fgsec.halogen.features.playlists.PlaylistDetailView
import org.fgsec.halogen.features.podcasts.EpisodesView
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.PlaylistData
import org.fgsec.halogen.wire.PodcastData

/// Typed navigation targets shared by every tab's NavHost — one vocabulary so
/// any list can push any detail without per-stack destination collisions (the
/// Kotlin analog of the web's Route enum, entity slice).
sealed interface AppRoute {
    @Serializable
    data class Podcast(val id: Int) : AppRoute

    @Serializable
    data class Episode(val id: Int) : AppRoute

    @Serializable
    data class Playlist(val id: Int) : AppRoute

    /// The episode metadata (raw key/value) screen — a route because it's
    /// pushed from a menu, and menu items can't host their own navigation.
    @Serializable
    data class EpisodeMetadata(val id: Int) : AppRoute
}

/// Per-tab programmatic navigation: rows and menu items push routes through this
/// instead of embedding navigation in row markup (own-frame hit-testing; menus can
/// navigate). Each tab owns one Navigator, so back stacks stay per-tab.
class Navigator(private val controller: NavHostController) {
    fun push(route: AppRoute) {
        controller.navigate(route)
    }

    fun pop() {
        controller.popBackStack()
    }
}

/// The active tab's Navigator — rows/menus read this instead of taking a
/// navigator parameter (the iOS @Environment(Navigator.self) shape).
val LocalNavigator = staticCompositionLocalOf<Navigator?> { null }

/// Register the shared destinations on a tab's NavHost graph.
fun NavGraphBuilder.appDestinations(core: HalogenCore, models: Models) {
    composable<AppRoute.Podcast> { entry ->
        PodcastScreen(id = entry.toRoute<AppRoute.Podcast>().id, core = core, models = models)
    }
    composable<AppRoute.Episode> { entry ->
        val navigator = LocalNavigator.current
        EpisodeDetailView(
            core = core,
            episodeId = entry.toRoute<AppRoute.Episode>().id,
            onBack = { navigator?.pop() },
        )
    }
    composable<AppRoute.Playlist> { entry ->
        PlaylistScreen(id = entry.toRoute<AppRoute.Playlist>().id, core = core, models = models)
    }
    composable<AppRoute.EpisodeMetadata> { entry ->
        val id = entry.toRoute<AppRoute.EpisodeMetadata>().id
        JsonKeyValueView(
            title = "Metadata", core = core, path = "episodes/$id",
            cacheKey = CacheKey.metadata("episodes/$id"))
    }
}

/// Resolves a podcast id local-first (pool, then offline snapshot, then network);
/// a network hit upserts into pool + snapshot so the NEXT visit renders instantly and offline.
@Composable
private fun PodcastScreen(id: Int, core: HalogenCore, models: Models) {
    var resolved by remember { mutableStateOf<PodcastData?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    suspend fun resolve() {
        // Offline snapshot (everything the library list ever fetched).
        val cached = core.store?.load<List<PodcastData>>(CacheKey.podcasts)
        val hit = cached?.firstOrNull { it.id == id }
        if (hit != null) {
            resolved = hit
            // Stale-while-revalidate: freshen the row in the background.
            scope.launch {
                val fresh = try {
                    core.podcast(id)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    DeviceLog.warn(
                        "PodcastScreen: background refresh failed — ${e::class.simpleName}: ${e.message}")
                    null
                }
                if (fresh != null) {
                    models.podcasts.upsert(fresh)
                    resolved = fresh
                }
            }
            return
        }
        // Genuine miss: fetch-through, then cache for next time (web:
        // get_podcast + cache_podcasts on the deep-link path).
        try {
            val fresh = core.podcast(id)
            models.podcasts.upsert(fresh)
            resolved = fresh
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            failure = if (core.isOffline)
                "This podcast hasn't been cached on this device yet."
            else FriendlyError.message(e)
        }
    }

    val podcast = models.podcasts.podcasts.firstOrNull { it.id == id } ?: resolved
    val currentFailure = failure
    when {
        podcast != null -> EpisodesView(core = core, podcast = podcast)
        currentFailure != null -> ResolveFailureView(
            title = if (core.isOffline) "Podcast not cached" else "Couldn't load podcast",
            systemImage = if (core.isOffline) "wifi.slash" else "waveform.circle",
            message = currentFailure,
            onRetry = { failure = null },
        )
        else -> {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            LaunchedEffect(Unit) { resolve() }
        }
    }
    // An offline miss recovers on its own once connectivity returns (web: the
    // deep-link hook re-runs on a later publish).
    LaunchedEffect(core.isOffline) {
        if (!core.isOffline && failure != null) failure = null
    }
}

/// Same local-first resolution for playlists: pool → snapshot → network, with
/// the fetched row upserted for offline re-visits.
@Composable
private fun PlaylistScreen(id: Int, core: HalogenCore, models: Models) {
    var resolved by remember { mutableStateOf<PlaylistData?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val navigator = LocalNavigator.current

    suspend fun resolve() {
        val cached = core.store?.load<List<PlaylistData>>(CacheKey.playlists)
        val hit = cached?.firstOrNull { it.id == id }
        if (hit != null) {
            resolved = hit
            scope.launch {
                val fresh = try {
                    core.playlist(id)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    DeviceLog.warn(
                        "PlaylistScreen: background refresh failed — ${e::class.simpleName}: ${e.message}")
                    null
                }
                if (fresh != null) {
                    models.playlists.upsert(fresh)
                    resolved = fresh
                }
            }
            return
        }
        try {
            val fresh = core.playlist(id)
            models.playlists.upsert(fresh)
            resolved = fresh
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            failure = if (core.isOffline)
                "This playlist hasn't been cached on this device yet."
            else FriendlyError.message(e)
        }
    }

    val playlist = models.playlists.playlists.firstOrNull { it.id == id } ?: resolved
    val currentFailure = failure
    when {
        playlist != null ->
            PlaylistDetailView(core = core, playlist = playlist, onBack = { navigator?.pop() })
        currentFailure != null -> ResolveFailureView(
            title = if (core.isOffline) "Playlist not cached" else "Couldn't load playlist",
            systemImage = if (core.isOffline) "wifi.slash" else "music.note.list",
            message = currentFailure,
            onRetry = { failure = null },
        )
        else -> {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            LaunchedEffect(Unit) { resolve() }
        }
    }
    LaunchedEffect(core.isOffline) {
        if (!core.isOffline && failure != null) failure = null
    }
}

/// The by-id screens' failure state (iOS ContentUnavailableView shape):
/// icon + title + monospaced detail + a Try again that re-arms the resolve.
@Composable
private fun ResolveFailureView(
    title: String,
    systemImage: String,
    message: String,
    onRetry: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon(systemImage),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(48.dp),
        )
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(
            message,
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Button(onClick = onRetry) { Text("Try again") }
    }
}
