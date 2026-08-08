package org.fgsec.halogen.features.admin

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.ConfigOverridesData

/// The input a parameter renders as (web: InputKind).
enum class ParamKind { Number, Percent, Bool, Date, Text }

/// Static metadata for one overridable parameter — `key` mirrors the
/// ConfigOverridesData field name exactly.
data class ParamMeta(val key: String, val label: String, val desc: String, val kind: ParamKind)

/// One editable override row in the working set.
data class OverrideRow(val key: String, val value: String, val error: String? = null)

/// The full overridable allowlist + typed value (de)serialization — mirrors
/// the web's PARAMS registry (params.rs), key-for-key.
object ConfigOverrideParams {
    val params: List<ParamMeta> = listOf(
        ParamMeta(
            "subscription_fallback_poll_interval_secs",
            "Fallback poll interval (secs)", "Default feed poll cadence", ParamKind.Number),
        ParamMeta(
            "subscription_poll_wake_interval_secs",
            "Poll wake interval (secs)", "How often the poller wakes", ParamKind.Number),
        ParamMeta(
            "subscription_fallback_max_episodes",
            "Fallback max episodes", "Max episodes fetched per feed", ParamKind.Number),
        ParamMeta(
            "subscription_max_concurrent_downloads",
            "Max concurrent downloads", "Parallel episode downloads", ParamKind.Number),
        ParamMeta(
            "subscription_max_poll_concurrent",
            "Max concurrent polls", "Parallel feed polls", ParamKind.Number),
        ParamMeta(
            "subscription_poll_auto_download_enabled",
            "Auto-download on poll", "Server-side auto-download default", ParamKind.Bool),
        ParamMeta(
            "subscription_auto_playlist_add_to_start",
            "Auto-add to start of playlists",
            "Insert auto-added episodes at the start", ParamKind.Bool),
        ParamMeta(
            "subscription_no_sync_before",
            "No sync before", "Ignore episodes before this date", ParamKind.Date),
        ParamMeta(
            "subscription_sync_on_start",
            "Sync on start", "Sync feeds at server boot", ParamKind.Bool),
        ParamMeta(
            "auth_token_expiry_minutes",
            "Token expiry (mins)", "JWT lifetime in minutes", ParamKind.Number),
        ParamMeta(
            "episode_playback_complete_percentage",
            "Playback complete %", "Mark finished within the last N%", ParamKind.Percent),
        ParamMeta(
            "opml_file",
            "OPML file", "Server path to a seed OPML file", ParamKind.Text),
    )

    fun meta(key: String): ParamMeta? = params.firstOrNull { it.key == key }

    /// The current (set) value of `key` in `data`, stringified, or null if unset.
    fun currentValue(data: ConfigOverridesData, key: String): String? = when (key) {
        "subscription_fallback_poll_interval_secs" ->
            data.subscription_fallback_poll_interval_secs?.toString()
        "subscription_poll_wake_interval_secs" ->
            data.subscription_poll_wake_interval_secs?.toString()
        "subscription_fallback_max_episodes" ->
            data.subscription_fallback_max_episodes?.toString()
        "subscription_max_concurrent_downloads" ->
            data.subscription_max_concurrent_downloads?.toString()
        "subscription_max_poll_concurrent" ->
            data.subscription_max_poll_concurrent?.toString()
        "subscription_poll_auto_download_enabled" ->
            data.subscription_poll_auto_download_enabled?.let { if (it) "true" else "false" }
        "subscription_auto_playlist_add_to_start" ->
            data.subscription_auto_playlist_add_to_start?.let { if (it) "true" else "false" }
        "subscription_no_sync_before" -> data.subscription_no_sync_before
        "subscription_sync_on_start" ->
            data.subscription_sync_on_start?.let { if (it) "true" else "false" }
        "auth_token_expiry_minutes" -> data.auth_token_expiry_minutes?.toString()
        "episode_playback_complete_percentage" ->
            data.episode_playback_complete_percentage?.toString()
        "opml_file" -> data.opml_file
        else -> null
    }

    /// Parse `raw` and set it into `data` under `key`; returns a short inline
    /// error message on a parse/range failure.
    fun applyValue(data: ConfigOverridesData, key: String, rawInput: String): String? {
        val raw = rawInput.trim()
        val whole = "Enter a whole number"
        when (key) {
            "subscription_fallback_poll_interval_secs" -> {
                val v = raw.toULongOrNull() ?: return whole
                data.subscription_fallback_poll_interval_secs = v
            }
            "subscription_poll_wake_interval_secs" -> {
                val v = raw.toULongOrNull() ?: return whole
                data.subscription_poll_wake_interval_secs = v
            }
            "subscription_fallback_max_episodes" -> {
                val v = raw.toUIntOrNull() ?: return whole
                data.subscription_fallback_max_episodes = v
            }
            "subscription_max_concurrent_downloads" -> {
                val v = raw.toUIntOrNull() ?: return whole
                data.subscription_max_concurrent_downloads = v
            }
            "subscription_max_poll_concurrent" -> {
                val v = raw.toUIntOrNull() ?: return whole
                data.subscription_max_poll_concurrent = v
            }
            "subscription_poll_auto_download_enabled" -> {
                val v = parseBool(raw) ?: return "Choose enabled or disabled"
                data.subscription_poll_auto_download_enabled = v
            }
            "subscription_auto_playlist_add_to_start" -> {
                val v = parseBool(raw) ?: return "Choose enabled or disabled"
                data.subscription_auto_playlist_add_to_start = v
            }
            "subscription_no_sync_before" -> {
                if (!isDate(raw)) return "Use YYYY-MM-DD"
                data.subscription_no_sync_before = raw
            }
            "subscription_sync_on_start" -> {
                val v = parseBool(raw) ?: return "Choose enabled or disabled"
                data.subscription_sync_on_start = v
            }
            "auth_token_expiry_minutes" -> {
                val v = raw.toULongOrNull() ?: return whole
                data.auth_token_expiry_minutes = v
            }
            "episode_playback_complete_percentage" -> {
                val v = raw.toULongOrNull() ?: return whole
                if (v > 100u) return "Must be between 0 and 100"
                data.episode_playback_complete_percentage = v.toUShort()
            }
            "opml_file" -> {
                if (raw.isEmpty()) return "Enter a path"
                data.opml_file = raw
            }
            else -> return "Unknown parameter"
        }
        return null
    }

    private fun parseBool(raw: String): Boolean? = when (raw) {
        "true" -> true
        "false" -> false
        else -> null
    }

    /// Lenient `YYYY-MM-DD` shape check (the server re-parses).
    private fun isDate(raw: String): Boolean {
        if (raw.length != 10) return false
        return raw.withIndex().all { (i, c) ->
            if (i == 4 || i == 7) c == '-' else c in '0'..'9'
        }
    }
}

/// Config-overrides editor (admin): typed editing of the allowlisted
/// parameters (wire ConfigOverridesData). POST replaces the set wholesale;
/// changes apply after a server restart — the web's config-overrides page
/// (crates/ui-views config_overrides_form) key-for-key.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConfigOverridesView(core: HalogenCore, onBack: (() -> Unit)? = null) {
    var rows by remember { mutableStateOf(listOf<OverrideRow>()) }
    var query by remember { mutableStateOf("") }
    var loaded by remember { mutableStateOf(false) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var serverError by remember { mutableStateOf<String?>(null) }
    var overridesDisabled by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var confirmSave by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val mutateDisabled = busy || core.isOffline || overridesDisabled

    suspend fun load() {
        if (!loaded) {
            try {
                val data = core.configOverrides()
                rows = ConfigOverrideParams.params.mapNotNull { p ->
                    ConfigOverrideParams.currentValue(data, p.key)?.let {
                        OverrideRow(key = p.key, value = it)
                    }
                }
                loaded = true
                loadError = null
            } catch (e: Exception) {
                DeviceLog.warn(
                    "ConfigOverridesView: refresh failed — ${e::class.simpleName}: ${e.message}")
                loadError = FriendlyError.message(e)
            }
        }
        // Best-effort: proactively learn whether the mechanism is disabled.
        runCatching { core.rawJson("admin/config") }.getOrNull()?.let { cfg ->
            (cfg["config_overrides_disabled"] as? JsonPrimitive)?.booleanOrNull?.let {
                overridesDisabled = it
            }
        }
    }

    /// Parse every row into typed data; annotate rows with inline errors.
    fun validate(): Boolean {
        val data = ConfigOverridesData()
        var ok = true
        rows = rows.map { row ->
            val err = ConfigOverrideParams.applyValue(data, row.key, row.value)
            if (err != null) ok = false
            row.copy(error = err)
        }
        return ok
    }

    suspend fun save() {
        val data = ConfigOverridesData()
        for (row in rows) {
            if (ConfigOverrideParams.applyValue(data, row.key, row.value) != null) return
        }
        busy = true
        try {
            core.setConfigOverrides(data)
            serverError = null
            ToastCenter.success("Overrides saved — restart the server for them to take effect.")
            onBack?.invoke()
        } catch (e: Exception) {
            serverError = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    suspend fun clearAll() {
        busy = true
        try {
            core.clearConfigOverrides()
            rows = emptyList()
            serverError = null
            ToastCenter.success("All overrides cleared — restart the server to apply.")
            onBack?.invoke()
        } catch (e: Exception) {
            serverError = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    LaunchedEffect(Unit) { load() }

    val suggestions = remember(rows, query) {
        val active = rows.map { it.key }.toSet()
        val q = query.trim().lowercase()
        ConfigOverrideParams.params.filter { p ->
            if (p.key in active) false
            else q.isEmpty() || p.label.lowercase().contains(q)
                || p.desc.lowercase().contains(q) || p.key.contains(q)
        }
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Config overrides") },
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
                    loaded = false
                    load()
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            LazyColumn(Modifier.fillMaxSize()) {
                if (overridesDisabled) {
                    item(key = "disabled-banner") {
                        Text(
                            "Config overrides are disabled on this server; changes can't be saved.",
                            style = MaterialTheme.typography.bodySmall,
                            color = halogenExtras.warning,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                    }
                }
                val failure = loadError
                if (failure != null) {
                    item(key = "load-error") {
                        Text(
                            "Could not load overrides: $failure",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                    }
                } else {
                    // Add an override.
                    item(key = "add-header") { OverridesSectionHeader("Add an override") }
                    item(key = "add-search") {
                        OutlinedTextField(
                            value = query,
                            onValueChange = { query = it },
                            placeholder = { Text("Search parameters…") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(
                                capitalization = KeyboardCapitalization.None,
                                autoCorrectEnabled = false,
                            ),
                            modifier = Modifier.fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                    }
                    if (suggestions.isEmpty()) {
                        item(key = "no-suggestions") {
                            Text(
                                "No matching parameters.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                        }
                    } else {
                        items(suggestions, key = { "suggest-${it.key}" }) { p ->
                            Column(
                                Modifier.fillMaxWidth()
                                    .clickable {
                                        // Booleans default to Enabled (a valid
                                        // value); others start empty (web parity).
                                        rows = rows + OverrideRow(
                                            key = p.key,
                                            value = if (p.kind == ParamKind.Bool) "true" else "")
                                        query = ""
                                    }
                                    .padding(horizontal = 16.dp, vertical = 8.dp),
                                verticalArrangement = Arrangement.spacedBy(1.dp),
                            ) {
                                Text(p.label, style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    p.desc,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }

                    // Active overrides.
                    item(key = "active-header") { OverridesSectionHeader("Active overrides") }
                    if (rows.isEmpty()) {
                        item(key = "no-rows") {
                            Text(
                                "No overrides set yet — search above to add one.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                        }
                    }
                    items(rows, key = { "row-${it.key}" }) { row ->
                        DismissableOverrideRow(
                            row = row,
                            onChange = { updated ->
                                rows = rows.map { if (it.key == updated.key) updated else it }
                            },
                            onRemove = { rows = rows.filterNot { it.key == row.key } },
                        )
                    }
                    if (rows.isNotEmpty()) {
                        item(key = "active-footer") {
                            Text(
                                "Swipe to remove one, then Save to apply the new set.",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                            )
                        }
                    }

                    val srvError = serverError
                    if (srvError != null) {
                        item(key = "server-error") {
                            Text(
                                srvError,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                        }
                    }

                    item(key = "actions") {
                        Column(
                            Modifier.fillMaxWidth().padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Button(
                                onClick = {
                                    serverError = null
                                    if (validate()) confirmSave = true
                                },
                                enabled = !mutateDisabled,
                                modifier = Modifier.fillMaxWidth(),
                            ) { Text("Save overrides") }
                            OutlinedButton(
                                onClick = {
                                    serverError = null
                                    confirmClear = true
                                },
                                enabled = !mutateDisabled,
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text("Clear all", color = MaterialTheme.colorScheme.error)
                            }
                            Text(
                                "Overrides are saved to the server's overrides file and take effect after a restart.",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }

    if (confirmSave) {
        AlertDialog(
            onDismissRequest = { confirmSave = false },
            title = { Text("Save config overrides?") },
            text = {
                Text("This replaces the server's overrides file with the current set. Restart the server afterwards to apply them.")
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmSave = false
                    scope.launch { save() }
                }) { Text("Save") }
            },
            dismissButton = {
                TextButton(onClick = { confirmSave = false }) { Text("Cancel") }
            },
        )
    }
    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            title = { Text("Clear all overrides?") },
            text = {
                Text("This deletes every override and reverts the server to its configured defaults. Restart afterwards to apply.")
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmClear = false
                    scope.launch { clearAll() }
                }) { Text("Clear all", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmClear = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun OverridesSectionHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

/// iOS `.onDelete` parity: swipe an active row away to remove it from the
/// working set (Save then applies the new set wholesale).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DismissableOverrideRow(
    row: OverrideRow,
    onChange: (OverrideRow) -> Unit,
    onRemove: () -> Unit,
) {
    val dismissState = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            if (value == SwipeToDismissBoxValue.EndToStart) {
                onRemove()
                true
            } else false
        },
    )
    SwipeToDismissBox(
        state = dismissState,
        enableDismissFromStartToEnd = false,
        backgroundContent = {
            Box(
                Modifier.fillMaxSize()
                    .background(MaterialTheme.colorScheme.error)
                    .padding(horizontal = 16.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Icon(
                    halogenIcon("trash"),
                    contentDescription = "Remove",
                    tint = MaterialTheme.colorScheme.onError,
                )
            }
        },
    ) {
        Box(Modifier.background(MaterialTheme.colorScheme.surface)) {
            OverrideRowView(row = row, onChange = onChange)
        }
    }
}

/// One active override: label/description + the type-appropriate input + the
/// inline parse error.
@Composable
private fun OverrideRowView(row: OverrideRow, onChange: (OverrideRow) -> Unit) {
    val meta = ConfigOverrideParams.meta(row.key)
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            meta?.label ?: row.key,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.Medium),
        )
        if (!meta?.desc.isNullOrEmpty()) {
            Text(
                meta?.desc ?: "",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when (meta?.kind ?: ParamKind.Text) {
            ParamKind.Bool -> Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Enabled", Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                Switch(
                    checked = row.value == "true",
                    onCheckedChange = {
                        onChange(row.copy(value = if (it) "true" else "false", error = null))
                    },
                )
            }
            ParamKind.Number, ParamKind.Percent -> OverrideTextField(
                row, onChange, placeholder = "0",
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            )
            ParamKind.Date -> OverrideTextField(
                row, onChange, placeholder = "YYYY-MM-DD",
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
            )
            ParamKind.Text -> OverrideTextField(
                row, onChange, placeholder = "Value",
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
            )
        }
        row.error?.let { error ->
            Text(
                error,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}

@Composable
private fun OverrideTextField(
    row: OverrideRow,
    onChange: (OverrideRow) -> Unit,
    placeholder: String,
    keyboardOptions: KeyboardOptions,
) {
    OutlinedTextField(
        value = row.value,
        onValueChange = { onChange(row.copy(value = it, error = null)) },
        placeholder = { Text(placeholder) },
        singleLine = true,
        keyboardOptions = keyboardOptions,
        textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
        modifier = Modifier.fillMaxWidth(),
    )
}
