package org.fgsec.halogen.features.podcasts

import org.fgsec.halogen.core.ensureQueued
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.PodcastConfigData
import org.fgsec.halogen.wire.PodcastConfigStoreData
import org.fgsec.halogen.wire.PodcastConfigUpdateData
import org.fgsec.halogen.wire.PodcastData

/// The shared default constants (crates/utils constants.rs) — what the
/// web prefills create forms and unset overrides with.
private const val DEFAULT_POLL_INTERVAL = 3600u
private const val DEFAULT_MAX_EPISODES = 50u
private const val DEFAULT_MAX_CONCURRENT = 3u
private const val DEFAULT_AUTO_DOWNLOAD = false

private fun parseField(raw: String): UInt? = raw.trim().ifEmpty { null }?.toUIntOrNull()

/// Per-podcast download/poll config (create or edit). The override fields are
/// REQUIRED whole numbers prefilled with server defaults — the update can't clear
/// a value to NULL, so the form always sends a full set; reverting to global
/// defaults is the Remove action, not an empty field (web podcast_config_form.rs).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PodcastConfigFormView(core: HalogenCore, podcast: PodcastData, onBack: (() -> Unit)? = null) {
    // Prefill with the shared defaults; on edit an existing override wins,
    // an unset field falls back to the default (web `fill`).
    val initial = podcast.podcast_config
    /// The config body driving edit-vs-create — starts from the cached row,
    /// backfilled by id when the row carries only the FK.
    var config by remember { mutableStateOf<PodcastConfigData?>(initial) }
    /// Web `edit_ready`: a podcast whose cached row has a config FK but no
    /// body must NOT render as "create" prefilled with defaults — Save would
    /// overwrite the real server config. Held false until the body loads.
    var editReady by remember {
        mutableStateOf(podcast.podcast_config_id == null || initial != null)
    }
    var pollInterval by remember {
        mutableStateOf((initial?.poll_interval_seconds ?: DEFAULT_POLL_INTERVAL).toString())
    }
    var maxEpisodes by remember {
        mutableStateOf((initial?.max_episodes ?: DEFAULT_MAX_EPISODES).toString())
    }
    var maxConcurrent by remember {
        mutableStateOf((initial?.max_concurrent_downloads ?: DEFAULT_MAX_CONCURRENT).toString())
    }
    var autoDownload by remember {
        mutableStateOf(initial?.auto_download_enabled ?: DEFAULT_AUTO_DOWNLOAD)
    }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // Validation (web validate_fields: whole number + range rules from
    // PodcastConfigStoreData::validate).
    val pollError = parseField(pollInterval).let { v ->
        when {
            v == null -> "Enter a whole number"
            v <= 86400u -> null
            else -> "Poll interval must be between 0 and 86400 seconds"
        }
    }
    val maxEpisodesError = parseField(maxEpisodes).let { v ->
        when {
            v == null -> "Enter a whole number"
            v in 1u..10000u -> null
            else -> "Max episodes must be between 1 and 10000"
        }
    }
    val maxConcurrentError = parseField(maxConcurrent).let { v ->
        when {
            v == null -> "Enter a whole number"
            v in 1u..100u -> null
            else -> "Max concurrent downloads must be between 1 and 100"
        }
    }
    val isValid = pollError == null && maxEpisodesError == null && maxConcurrentError == null

    /// Cached row carries the FK but not the body → fetch it before the form
    /// is editable (web podcast_config_form.rs: prefill by id, `initialized`
    /// only on success).
    suspend fun backfillConfig() {
        val configId = podcast.podcast_config_id ?: return
        if (editReady) return
        try {
            val fetched = core.podcastConfig(configId)
            config = fetched
            // An unset override falls back to the shared default (web `fill`).
            pollInterval = (fetched.poll_interval_seconds ?: DEFAULT_POLL_INTERVAL).toString()
            maxEpisodes = (fetched.max_episodes ?: DEFAULT_MAX_EPISODES).toString()
            maxConcurrent =
                (fetched.max_concurrent_downloads ?: DEFAULT_MAX_CONCURRENT).toString()
            autoDownload = fetched.auto_download_enabled ?: DEFAULT_AUTO_DOWNLOAD
            editReady = true
            error = null
        } catch (e: Exception) {
            error =
                "Couldn't load the current config — editing is disabled so a save can't overwrite it. $e"
        }
    }

    suspend fun save() {
        val p = parseField(pollInterval) ?: return
        val me = parseField(maxEpisodes) ?: return
        val mc = parseField(maxConcurrent) ?: return
        if (!isValid) return
        saving = true
        try {
            val existing = config
            if (existing != null) {
                // Always the FULL set — a nil field would mean "leave
                // unchanged" server-side, never "back to default".
                val data = PodcastConfigUpdateData(
                    poll_interval_seconds = p,
                    max_episodes = me,
                    max_concurrent_downloads = mc,
                    auto_download_enabled = autoDownload,
                )
                // Web rule: edits go direct online (so the form shows server
                // errors) and queue as a durable UpdatePodcastConfig offline
                // (an existing id is safe to drain later).
                if (core.isOffline) {
                    if (!core.ensureQueued(
                        OutboxOp.Kind.UpdatePodcastConfig(configId = existing.id, data = data))) return
                    onBack?.invoke()
                    return
                }
                core.updatePodcastConfig(configId = existing.id, data = data)
            } else {
                // Create stays online-only — it needs a real config id back
                // (web: the create submit is disabled offline).
                if (core.isOffline) {
                    error = "You're offline — reconnect to create a config."
                    return
                }
                core.createPodcastConfig(
                    podcastId = podcast.id,
                    data = PodcastConfigStoreData(
                        poll_interval_seconds = p,
                        max_episodes = me,
                        max_concurrent_downloads = mc,
                        auto_download_enabled = autoDownload,
                    ),
                )
            }
            core.models?.podcasts?.refresh()
            onBack?.invoke()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    suspend fun remove() {
        saving = true
        try {
            // Remove behaves like edit (an existing id): direct online,
            // durable RemovePodcastConfig op offline (web rule).
            if (core.isOffline) {
                if (!core.ensureQueued(OutboxOp.Kind.RemovePodcastConfig(podcastId = podcast.id))) return
                onBack?.invoke()
                return
            }
            core.deletePodcastConfig(podcastId = podcast.id)
            core.models?.podcasts?.refresh()
            onBack?.invoke()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    LaunchedEffect(Unit) { backfillConfig() }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Download config") },
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
        Column(
            Modifier.fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ConfigNumberField("Poll interval (secs)", pollInterval, pollError) {
                pollInterval = it
            }
            ConfigNumberField("Max episodes", maxEpisodes, maxEpisodesError) {
                maxEpisodes = it
            }
            ConfigNumberField("Max concurrent downloads", maxConcurrent, maxConcurrentError) {
                maxConcurrent = it
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Auto-download new episodes", Modifier.weight(1f))
                Switch(checked = autoDownload, onCheckedChange = { autoDownload = it })
            }
            Text(
                "Override how often this podcast is polled and how its episodes download. Leave the defaults to match the server; Remove reverts to the server-wide defaults.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            if (!editReady && error == null) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    Text(
                        "Loading current config…",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            error?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }

            Button(
                onClick = { scope.launch { save() } },
                enabled = !saving && isValid && editReady,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (saving) {
                    CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                } else {
                    Text("Save")
                }
            }

            if (podcast.podcast_config_id != null) {
                OutlinedButton(
                    onClick = { scope.launch { remove() } },
                    enabled = !saving,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        "Remove config (use defaults)",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
    }
}

@Composable
private fun ConfigNumberField(
    label: String,
    value: String,
    error: String?,
    onChange: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        OutlinedTextField(
            value = value,
            onValueChange = onChange,
            label = { Text(label) },
            singleLine = true,
            isError = error != null,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            modifier = Modifier.fillMaxWidth(),
        )
        error?.let {
            Text(
                it,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}
