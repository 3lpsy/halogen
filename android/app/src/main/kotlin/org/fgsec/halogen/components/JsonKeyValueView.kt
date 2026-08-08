package org.fgsec.halogen.components

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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// The cached row shape (pairs aren't wire-stable).
@Serializable
private data class CachedRow(val key: String, val value: String)

/// Generic read-only key→value page over any envelope endpoint. `cacheKey`: when
/// set, rows render local-first from that LocalStore key and fetches re-snapshot
/// it; null for genuinely network-only surfaces like View Config (web parity).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun JsonKeyValueView(
    title: String,
    core: HalogenCore,
    path: String,
    cacheKey: String? = null,
    onBack: (() -> Unit)? = null,
) {
    var values by remember { mutableStateOf(listOf<Pair<String, String>>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        if (values.isEmpty() && cacheKey != null) {
            core.store?.load<List<CachedRow>>(cacheKey)?.let { cached ->
                values = cached.map { it.key to it.value }
            }
        }
        try {
            val dict = core.rawJson(path)
            values = dict.map { (key, value) -> key to renderValue(value) }
                .sortedBy { it.first }
            error = null
            if (cacheKey != null) {
                core.store?.save(values.map { CachedRow(it.first, it.second) }, cacheKey)
            }
        } catch (e: Exception) {
            DeviceLog.warn("JsonKeyValueView: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (values.isEmpty()) error = FriendlyError.message(e)
        }
    }

    LaunchedEffect(path) { load() }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text(title) },
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
            when {
                failure != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(32.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Icon(
                        halogenIcon("wifi.exclamationmark"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text("Couldn't load", style = MaterialTheme.typography.titleMedium)
                    Text(
                        failure,
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontFamily = FontFamily.Monospace),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }

                values.isEmpty() -> Box(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
                    contentAlignment = Alignment.Center,
                ) { CircularProgressIndicator() }

                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(values, key = { it.first }) { (key, value) ->
                        Column(
                            Modifier.fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 6.dp),
                            verticalArrangement = Arrangement.spacedBy(2.dp),
                        ) {
                            Text(
                                key,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                value,
                                style = MaterialTheme.typography.bodyMedium.copy(
                                    fontFamily = FontFamily.Monospace),
                            )
                        }
                    }
                }
            }
        }
    }
}

/// Render one envelope value for display (iOS JSONKeyValueView.render parity).
private fun renderValue(value: JsonElement): String = when {
    value is JsonNull -> "—"
    value is JsonPrimitive && value.isString -> value.content.ifEmpty { "\"\"" }
    value is JsonPrimitive -> value.content  // true/false/number literals
    else -> Json.encodeToString(JsonElement.serializer(), sortedKeys(value))
}

/// Stable output for nested values: objects encode with sorted keys.
private fun sortedKeys(element: JsonElement): JsonElement = when (element) {
    is JsonObject -> JsonObject(
        element.entries.sortedBy { it.key }.associate { it.key to sortedKeys(it.value) })
    is JsonArray -> JsonArray(element.map(::sortedKeys))
    else -> element
}
