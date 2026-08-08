package org.fgsec.halogen.features.settings

import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.fgsec.halogen.components.ArtLoader
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.HalogenCore

/// Local Data (the web's cache-control page, native): everything stored on this
/// device with live size/count stats, each category individually deletable.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LocalDataView(core: HalogenCore, onBack: () -> Unit) {
    var stats by remember { mutableStateOf(LocalDataStats()) }
    var message by remember { mutableStateOf<String?>(null) }
    var confirmEmbedded by remember { mutableStateOf(false) }
    var confirmDeleteAll by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    suspend fun reload() {
        stats = LocalDataStats.collect(core)
    }

    LaunchedEffect(Unit) { reload() }

    fun size(bytes: Long): String = Formatter.formatFileSize(context, bytes)

    /// One purge row's tap: run the action, note the result, re-census.
    fun purge(action: suspend () -> String) {
        scope.launch {
            message = action()
            reload()
        }
    }

    /// The one-tap full reset (everything except the embedded library).
    /// Goes through the models so live state — device download statuses
    /// included — resets with the files.
    suspend fun deleteAll() {
        busy = true
        try {
            core.store?.remove(stats.contentKeys + stats.viewSettingsKeys + stats.prefsKeys)
            core.outbox?.clearAll()
            core.models?.device?.removeAll()
            ArtLoader.configure(core.appContext, core.apiToken, core.account?.namespace)
            DeviceLog.shared.clear()
            withContext(Dispatchers.IO) {
                LocalDataStats.removeOtherNamespaces(core.appContext, core.account?.namespace)
            }
            core.remountModels()
            message = "Deleted all local data"
            reload()
        } finally {
            busy = false
        }
    }

    suspend fun deleteEmbedded() {
        busy = true
        try {
            core.destroyEmbeddedServer()
            message = "Embedded server deleted"
            reload()
        } catch (e: Exception) {
            message = "Delete failed: $e"
        } finally {
            busy = false
        }
    }

    SettingsScaffold(title = "Local data", onBack = onBack) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    reload()
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                // Result feedback FIRST — at the bottom of this long list it
                // sat below the fold, so purges looked unacknowledged.
                message?.let {
                    SettingsGroup {
                        SettingsRow {
                            Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }

                SettingsGroup(header = "This account") {
                    PurgeRow(
                        "Cached lists",
                        detail = "${stats.contentCount} items · ${size(stats.contentBytes)}",
                        icon = "internaldrive",
                    ) {
                        purge {
                            core.store?.remove(stats.contentKeys)
                            core.remountModels()
                            "Cleared the content cache"
                        }
                    }
                    PurgeRow(
                        "View settings",
                        detail = "${stats.viewSettingsCount} items",
                        icon = "slider.horizontal.3",
                    ) {
                        purge {
                            core.store?.remove(stats.viewSettingsKeys)
                            core.remountModels()
                            "Reset view settings"
                        }
                    }
                    PurgeRow(
                        "Preferences",
                        detail = "${stats.prefsCount} items",
                        icon = "gearshape",
                    ) {
                        purge {
                            core.store?.remove(stats.prefsKeys)
                            core.remountModels()
                            "Reset preferences"
                        }
                    }
                    PurgeRow(
                        "Pending sync queue",
                        detail = "${stats.outboxCount} ops",
                        icon = "arrow.triangle.2.circlepath",
                    ) {
                        purge {
                            core.outbox?.clearAll()
                            "Discarded the pending sync queue"
                        }
                    }
                    PurgeRow(
                        "Device audio",
                        detail = "${stats.audioCount} files · ${size(stats.audioBytes)}",
                        icon = "arrow.down.circle",
                    ) {
                        purge {
                            core.models?.device?.removeAll()
                            "Deleted device audio"
                        }
                    }
                    PurgeRow(
                        "Artwork cache (memory)",
                        detail = "in-memory",
                        icon = "photo",
                    ) {
                        purge {
                            ArtLoader.configure(
                                core.appContext, core.apiToken, core.account?.namespace)
                            "Cleared artwork cache"
                        }
                    }
                    PurgeRow(
                        "Device log",
                        detail = "${DeviceLog.shared.entries.size} entries",
                        icon = "doc.text",
                    ) {
                        purge {
                            DeviceLog.shared.clear()
                            "Cleared the device log"
                        }
                    }
                }

                SettingsGroup(header = "Other accounts") {
                    PurgeRow(
                        "Other accounts' data",
                        detail = "${stats.otherAccounts} accounts · ${size(stats.otherBytes)}",
                        icon = "person.2",
                    ) {
                        purge {
                            withContext(Dispatchers.IO) {
                                LocalDataStats.removeOtherNamespaces(
                                    core.appContext, core.account?.namespace)
                            }
                            "Deleted other accounts' local data"
                        }
                    }
                }

                SettingsGroup(
                    footer = "Clears every cache, setting, pending sync op, and downloaded file for all accounts on this device. The embedded server library below is separate.",
                ) {
                    SettingsRow(onClick = { confirmDeleteAll = true }, enabled = !busy) {
                        Text("Delete all local data…", color = MaterialTheme.colorScheme.error)
                    }
                }

                SettingsGroup(
                    header = "Embedded server",
                    footer = "Deletes this device's entire library — database, media files, and users. Remote servers are unaffected.",
                ) {
                    SettingsRow {
                        Icon(
                            halogenIcon("externaldrive.badge.xmark"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text("Embedded server data", Modifier.weight(1f))
                        RowValue(size(stats.embeddedBytes))
                    }
                    SettingsRow(
                        onClick = { confirmEmbedded = true },
                        enabled = !busy && stats.embeddedBytes != 0L,
                    ) {
                        Text(
                            "Delete embedded server…",
                            color = if (!busy && stats.embeddedBytes != 0L)
                                MaterialTheme.colorScheme.error
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }

    if (confirmEmbedded) {
        AlertDialog(
            onDismissRequest = { confirmEmbedded = false },
            title = { Text("Delete the embedded server?") },
            text = { Text("The database, media, and every device user are permanently removed.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmEmbedded = false
                    // App scope: destroy signs out and unmounts this view —
                    // a composition scope would cancel the purge mid-way.
                    core.scope.launch { deleteEmbedded() }
                }) { Text("Delete everything", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmEmbedded = false }) { Text("Cancel") }
            },
        )
    }
    if (confirmDeleteAll) {
        AlertDialog(
            onDismissRequest = { confirmDeleteAll = false },
            title = { Text("Delete all local data?") },
            text = {
                Text(
                    "Caches, view settings, preferences, pending sync ops, device audio, and other accounts' data are permanently removed. The embedded server library is not touched."
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmDeleteAll = false
                    core.scope.launch { deleteAll() }
                }) { Text("Delete all", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmDeleteAll = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun PurgeRow(
    title: String,
    detail: String,
    icon: String,
    onDelete: () -> Unit,
) {
    SettingsRow {
        Icon(
            halogenIcon(icon),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(title, Modifier.weight(1f))
        RowValue(detail)
        IconButton(onClick = onDelete) {
            Icon(
                halogenIcon("trash"),
                contentDescription = "Delete $title",
                tint = MaterialTheme.colorScheme.error,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

/// The census behind the Local Data screen.
data class LocalDataStats(
    val contentKeys: List<String> = emptyList(),
    val viewSettingsKeys: List<String> = emptyList(),
    val prefsKeys: List<String> = emptyList(),
    val contentCount: Int = 0,
    val contentBytes: Long = 0,
    val viewSettingsCount: Int = 0,
    val prefsCount: Int = 0,
    val outboxCount: Int = 0,
    val audioCount: Int = 0,
    val audioBytes: Long = 0,
    val otherAccounts: Int = 0,
    val otherBytes: Long = 0,
    val embeddedBytes: Long = 0,
) {
    companion object {
        /// Keys that count as "Preferences" (same set as the iOS census).
        val prefKeys: Set<String> = setOf("nav-config", "swipe-prefs", "client-prefs")

        suspend fun collect(core: HalogenCore): LocalDataStats {
            val store = core.store
            val contentKeys = mutableListOf<String>()
            val viewSettingsKeys = mutableListOf<String>()
            val prefsKeys = mutableListOf<String>()
            if (store != null) {
                for (key in store.listKeys()) {
                    when {
                        key in Companion.prefKeys -> prefsKeys.add(key)
                        key.startsWith("listquery-") || key.endsWith("-scroll-anchor") ->
                            viewSettingsKeys.add(key)
                        key == "outbox" -> {} // counted below from the live outbox
                        else -> contentKeys.add(key)
                    }
                }
            }
            val contentBytes = store?.bytes(contentKeys)?.toLong() ?: 0L
            val outboxCount = core.outbox?.pendingCount() ?: 0
            val deviceStats = core.models?.device?.stats
            val (others, otherBytes, embeddedBytes) = withContext(Dispatchers.IO) {
                val (count, bytes) = otherNamespaceStats(core.appContext, core.account?.namespace)
                Triple(count, bytes, directoryBytes(core.embeddedRoot()))
            }
            return LocalDataStats(
                contentKeys = contentKeys,
                viewSettingsKeys = viewSettingsKeys,
                prefsKeys = prefsKeys,
                contentCount = contentKeys.size,
                contentBytes = contentBytes,
                viewSettingsCount = viewSettingsKeys.size,
                prefsCount = prefsKeys.size,
                outboxCount = outboxCount,
                audioCount = deviceStats?.count ?: 0,
                audioBytes = deviceStats?.bytes ?: 0L,
                otherAccounts = others,
                otherBytes = otherBytes,
                embeddedBytes = embeddedBytes,
            )
        }

        // ── filesystem census helpers ────────────────────────────────────────

        fun clientRoot(context: Context): File = File(context.filesDir, "halogen-client")

        fun otherNamespaceStats(context: Context, active: String?): Pair<Int, Long> {
            val names = (clientRoot(context).list() ?: emptyArray()).filter { it != active }
            val bytes = names.sumOf { directoryBytes(File(clientRoot(context), it)) }
            return names.size to bytes
        }

        fun removeOtherNamespaces(context: Context, active: String?) {
            val root = clientRoot(context)
            for (name in (root.list() ?: emptyArray()).filter { it != active }) {
                File(root, name).deleteRecursively()
            }
        }

        fun directoryBytes(dir: File): Long =
            if (!dir.exists()) 0L
            else dir.walkTopDown().filter { it.isFile }.sumOf { it.length() }
    }
}
