package org.fgsec.halogen.features.more

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.BuiltinNav
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.Models
import org.fgsec.halogen.features.admin.ConfigOverridesView
import org.fgsec.halogen.features.admin.DeviceLogsView
import org.fgsec.halogen.features.admin.PollingView
import org.fgsec.halogen.features.admin.ServerLogsView
import org.fgsec.halogen.features.discover.DiscoverView
import org.fgsec.halogen.features.downloads.DownloadsView
import org.fgsec.halogen.features.history.HistoryView
import org.fgsec.halogen.features.latest.LatestView
import org.fgsec.halogen.features.playlists.PlaylistsView
import org.fgsec.halogen.features.podcasts.PodcastsView
import org.fgsec.halogen.features.queue.QueueView
import org.fgsec.halogen.features.settings.SettingsView

/// The dock's always-present More tab — the full navigation menu: every
/// visible destination in configured order. Destinations not built natively
/// yet are listed disabled so the gap is visible, not silent.
@Composable
fun MoreView(core: HalogenCore, models: Models, onOpen: (BuiltinNav) -> Unit) {
    androidx.compose.foundation.layout.Column {
        HalogenNavbar(core)
        LazyColumn {
            items(models.nav.visibleItems(isAdmin = core.isAdmin)) { item ->
                val implemented = isImplemented(item)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .then(
                            if (implemented) Modifier.clickable { onOpen(item) }
                            else Modifier
                        )
                        .padding(horizontal = 16.dp, vertical = 14.dp)
                        .testTag("more-${item.token}"),
                ) {
                    val tint = if (implemented) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.outline
                    Icon(halogenIcon(item.systemImage), contentDescription = null, tint = tint)
                    Spacer(Modifier.padding(6.dp))
                    Text(
                        item.label,
                        color = if (implemented) MaterialTheme.colorScheme.onSurface
                        else MaterialTheme.colorScheme.outline,
                    )
                    Spacer(Modifier.weight(1f))
                    if (!implemented) {
                        Text(
                            "Soon", style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.outline,
                        )
                    } else {
                        Icon(
                            halogenIcon("chevron.right"), contentDescription = null,
                            tint = MaterialTheme.colorScheme.outline,
                        )
                    }
                }
            }
            item {
                Text(
                    "Reorder or hide items in Settings → Configure dock.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
    }
}

/// Every destination is built on Android (parity with iOS Round G+).
private fun isImplemented(item: BuiltinNav): Boolean = true

/// Maps a nav destination to its screen — shared by the dock tabs and the
/// More menu so both always agree on what exists.
@Composable
fun BuiltinDestination(item: BuiltinNav, core: HalogenCore, models: Models) {
    when (item) {
        BuiltinNav.Queue -> QueueView(models.queue, core)
        BuiltinNav.Latest -> LatestView(models.latest, core)
        BuiltinNav.Podcasts -> PodcastsView(models.podcasts, core)
        BuiltinNav.Playlists -> PlaylistsView(models.playlists, core)
        BuiltinNav.Downloads -> DownloadsView(models.downloads, core)
        BuiltinNav.Settings -> SettingsView(core, models.nav)
        BuiltinNav.Discover -> DiscoverView(models.discover, core)
        BuiltinNav.History -> HistoryView(models.history, core)
        BuiltinNav.Polling -> PollingView(core)
        BuiltinNav.ServerLogs -> ServerLogsView(core)
        BuiltinNav.DeviceLogs -> DeviceLogsView(core)
    }
}
