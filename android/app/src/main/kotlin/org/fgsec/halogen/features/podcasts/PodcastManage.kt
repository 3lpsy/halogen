package org.fgsec.halogen.features.podcasts

import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import org.fgsec.halogen.components.JsonKeyValueView
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.PodcastData

/// Podcast management destinations — menu items can't push screens
/// themselves, so the podcast page and the library row menu both drive this
/// via a nullable route state (iOS `navigationDestination(item:)`).
enum class PodcastManageRoute(val rawValue: String) {
    Edit("edit"),
    DownloadConfig("downloadConfig"),
    AutoPlaylists("autoPlaylists"),
    Metadata("metadata"),
}

/// The shared menu items (podcast page toolbar + library row ellipsis) —
/// place inside a DropdownMenu; callers close the menu in the callbacks and
/// own the delete confirm (iOS: the confirmationDialog lives in callers).
@Composable
fun PodcastManageMenu(
    onManage: (PodcastManageRoute) -> Unit,
    onDelete: () -> Unit,
) {
    DropdownMenuItem(
        text = { Text("Edit podcast") },
        leadingIcon = { Icon(halogenIcon("pencil"), contentDescription = null) },
        onClick = { onManage(PodcastManageRoute.Edit) },
    )
    DropdownMenuItem(
        text = { Text("Download config") },
        leadingIcon = { Icon(halogenIcon("arrow.down.circle.dotted"), contentDescription = null) },
        onClick = { onManage(PodcastManageRoute.DownloadConfig) },
    )
    DropdownMenuItem(
        text = { Text("Auto-playlists") },
        leadingIcon = { Icon(halogenIcon("music.note.list"), contentDescription = null) },
        onClick = { onManage(PodcastManageRoute.AutoPlaylists) },
    )
    DropdownMenuItem(
        text = { Text("Metadata") },
        leadingIcon = { Icon(halogenIcon("info.circle"), contentDescription = null) },
        onClick = { onManage(PodcastManageRoute.Metadata) },
    )
    HorizontalDivider()
    DropdownMenuItem(
        text = { Text("Delete podcast") },
        leadingIcon = { Icon(halogenIcon("trash"), contentDescription = null) },
        colors = MenuDefaults.itemColors(
            textColor = MaterialTheme.colorScheme.error,
            leadingIconColor = MaterialTheme.colorScheme.error,
        ),
        onClick = onDelete,
    )
}

/// Resolves a manage route to its screen.
@Composable
fun PodcastManageScreen(
    route: PodcastManageRoute,
    core: HalogenCore,
    podcast: PodcastData,
    onBack: (() -> Unit)? = null,
) {
    when (route) {
        PodcastManageRoute.Edit -> PodcastEditView(core, podcast, onBack)
        PodcastManageRoute.DownloadConfig -> PodcastConfigFormView(core, podcast, onBack)
        PodcastManageRoute.AutoPlaylists -> AutoPlaylistsView(core, podcast, onBack)
        PodcastManageRoute.Metadata -> JsonKeyValueView(
            title = "Metadata",
            core = core,
            path = "podcasts/${podcast.id}",
            cacheKey = CacheKey.metadata("podcasts/${podcast.id}"),
            onBack = onBack,
        )
    }
}
