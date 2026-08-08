package org.fgsec.halogen.features.admin

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.ServerErrorsData

/// Server failure histories (admin): RSS sync + episode download errors —
/// reached from Settings, like the web.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerErrorsView(core: HalogenCore, onBack: (() -> Unit)? = null) {
    var data by remember { mutableStateOf<ServerErrorsData?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        try {
            data = core.serverErrors()
            error = null
        } catch (e: Exception) {
            DeviceLog.warn("ServerErrorsView: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (data == null) error = FriendlyError.message(e)
        }
    }

    LaunchedEffect(Unit) { load() }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Server errors") },
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
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    load()
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            val failure = error
            val loaded = data
            when {
                failure != null -> AdminUnavailableView(
                    icon = "wifi.exclamationmark",
                    title = "Couldn't load errors",
                    description = failure,
                    monospaced = true,
                )

                loaded == null -> Box(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
                    contentAlignment = Alignment.Center,
                ) { CircularProgressIndicator() }

                loaded.rss_sync.isEmpty() && loaded.episode_downloads.isEmpty() ->
                    AdminUnavailableView(
                        icon = "checkmark.seal",
                        title = "No server errors",
                        description = "Feed syncs and downloads are healthy.",
                    )

                else -> LazyColumn(Modifier.fillMaxSize()) {
                    if (loaded.rss_sync.isNotEmpty()) {
                        item(key = "feed-sync-header") { ErrorSectionHeader("Feed sync") }
                        items(loaded.rss_sync, key = { "rss-${it.id}" }) { e ->
                            ErrorRow(
                                title = e.podcast_title ?: "Podcast #${e.podcast_id}",
                                reason = e.reason,
                                createdAt = e.created_at,
                            )
                        }
                    }
                    if (loaded.episode_downloads.isNotEmpty()) {
                        item(key = "downloads-header") { ErrorSectionHeader("Episode downloads") }
                        items(loaded.episode_downloads, key = { "dl-${it.id}" }) { e ->
                            ErrorRow(
                                title = e.episode_title ?: "Episode #${e.episode_id}",
                                reason = e.reason,
                                createdAt = e.created_at,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ErrorSectionHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

@Composable
private fun ErrorRow(title: String, reason: String, createdAt: String) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.Medium),
        )
        Text(
            reason,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            formatJobDate(createdAt),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.outline,
        )
    }
}
