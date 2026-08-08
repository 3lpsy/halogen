package org.fgsec.halogen.features.podcasts

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.PodcastData

/// The cached auto-playlist selection (ids + insert-position override).
@Serializable
private data class AutoPlaylistsSnapshot(val playlistIds: List<Int>, val addToStart: Boolean?)

/// Which playlists this podcast auto-adds new episodes to. PUT replaces the whole
/// set, so editing is GATED until the current set has loaded — an unconfirmed empty
/// selection would wipe the server's real selections (web: `initialized` gate).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AutoPlaylistsView(core: HalogenCore, podcast: PodcastData, onBack: (() -> Unit)? = null) {
    var selected by remember { mutableStateOf(setOf<Int>()) }
    /// Insert-position override for auto-added episodes: true = start,
    /// false = end, null = server default (stamped on every link).
    var addToStart by remember { mutableStateOf<Boolean?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    /// The user touched the set this visit — a late network answer must not
    /// snap their toggles back (the queued outbox op is the newer truth).
    var edited by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    /// Durable replace-the-set op (web: SetPodcastAutoPlaylists) — the
    /// checkmarks above are the optimistic state; queues offline, and the
    /// FIFO outbox keeps rapid edits in order.
    fun save() {
        val ids = selected.toList()
        val position = addToStart
        scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.SetAutoPlaylists(
                    podcastId = podcast.id, playlistIds = ids, addToStart = position))
            // The snapshot tracks the queued truth, so an offline re-open
            // shows what will land on drain.
            core.store?.save(
                AutoPlaylistsSnapshot(playlistIds = ids, addToStart = position),
                CacheKey.autoPlaylists(podcast.id))
            error = null
        }
    }

    fun toggle(id: Int) {
        // Never edit an unconfirmed set — see the type doc.
        if (!loaded) return
        edited = true
        selected = if (id in selected) selected - id else selected + id
        save()
    }

    /// Local-first seed (web podcast_auto_playlists.rs: the cached set makes
    /// the screen readable AND editable offline; the PUT-wipe hazard the
    /// `loaded` gate protects against doesn't apply to a confirmed snapshot).
    suspend fun seedFromCache() {
        if (loaded) return
        val cached = core.store?.load<AutoPlaylistsSnapshot>(CacheKey.autoPlaylists(podcast.id))
            ?: return
        selected = cached.playlistIds.toSet()
        addToStart = cached.addToStart
        loaded = true
    }

    suspend fun load() {
        error = null
        try {
            val links = core.autoPlaylists(podcastId = podcast.id)
            if (!edited) {
                selected = links.map { it.playlist_id }.toSet()
                // Every link carries the same per-podcast override (the set
                // endpoint stamps it uniformly) — the first row speaks for
                // the set.
                addToStart = links.firstOrNull()?.add_to_start
            }
            loaded = true
            error = null
            core.store?.save(
                AutoPlaylistsSnapshot(
                    playlistIds = links.map { it.playlist_id },
                    addToStart = links.firstOrNull()?.add_to_start),
                CacheKey.autoPlaylists(podcast.id))
        } catch (e: Exception) {
            DeviceLog.warn("auto-playlists: load failed — ${e::class.simpleName}: ${e.message}")
            // A cache-seeded screen stays editable; only a cold miss blocks.
            if (!loaded) error = FriendlyError.message(e)
        }
    }

    LaunchedEffect(Unit) {
        core.models?.playlists?.load()
        seedFromCache()
        load()
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Auto-playlists") },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                        }
                    }
                },
            )
        },
    ) { padding ->
        val playlists = core.models?.playlists?.playlists ?: emptyList()
        LazyColumn(Modifier.fillMaxSize().padding(padding)) {
            if (!loaded && error == null) {
                item(key = "loading") {
                    Row(
                        Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                        Text(
                            "Loading auto-playlists…",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            items(playlists, key = { it.id }) { playlist ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable(enabled = loaded) { toggle(playlist.id) }
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(playlist.name)
                    if (playlist.is_default) {
                        Text(
                            "Queue",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Spacer(Modifier.weight(1f))
                    if (playlist.id in selected) {
                        Icon(
                            halogenIcon("checkmark"),
                            contentDescription = "Selected",
                            tint = MaterialTheme.colorScheme.primary,
                        )
                    }
                }
            }
            item(key = "footer") {
                Text(
                    "New episodes from ${podcast.title} are added to the checked playlists on each poll.",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
            item(key = "position") {
                InsertPositionRow(
                    addToStart = addToStart,
                    enabled = loaded,
                    onSelect = { newValue ->
                        addToStart = newValue
                        // The override rides every link, so a position change
                        // is a save of the same whole set.
                        if (loaded) {
                            edited = true
                            save()
                        }
                    },
                )
            }
            val failure = error
            if (failure != null) {
                item(key = "error") {
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            failure,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.weight(1f),
                        )
                        if (!loaded) {
                            TextButton(onClick = { scope.launch { load() } }) { Text("Retry") }
                        }
                    }
                }
            }
        }
    }
}

/// The insert-position picker (iOS Picker parity): only USER changes save —
/// the initial seed from load() must not fire a redundant PUT.
@Composable
private fun InsertPositionRow(
    addToStart: Boolean?,
    enabled: Boolean,
    onSelect: (Boolean?) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    val label = when (addToStart) {
        null -> "Server default"
        true -> "Start of playlist"
        false -> "End of playlist"
    }
    Row(
        Modifier.fillMaxWidth()
            .clickable(enabled = enabled) { open = true }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("New episodes are added to", Modifier.weight(1f))
        Box {
            Text(label, color = MaterialTheme.colorScheme.onSurfaceVariant)
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                for ((value, option) in listOf<Pair<Boolean?, String>>(
                    null to "Server default",
                    true to "Start of playlist",
                    false to "End of playlist",
                )) {
                    DropdownMenuItem(
                        text = { Text(option) },
                        trailingIcon = {
                            if (value == addToStart) {
                                Icon(halogenIcon("checkmark"), contentDescription = null)
                            }
                        },
                        onClick = {
                            open = false
                            if (value != addToStart) onSelect(value)
                        },
                    )
                }
            }
        }
    }
}
