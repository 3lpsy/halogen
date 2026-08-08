package org.fgsec.halogen.features.admin

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.ServerLogsData

/// Server log tail (admin) — the embedded/remote server's own application
/// log, parsed into the Device Logs presentation (level badge, time, source,
/// message), newest first, with search and manual refresh.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerLogsView(core: HalogenCore, onBack: (() -> Unit)? = null) {
    var logs by remember { mutableStateOf<ServerLogsData?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        loading = true
        try {
            logs = core.serverLogs()
            error = null
        } catch (e: Exception) {
            DeviceLog.warn("ServerLogsView: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (logs == null) error = FriendlyError.message(e)
        } finally {
            loading = false
        }
    }

    LaunchedEffect(Unit) { load() }

    // Newest first, like the web's tail; id = the original line index.
    val parsed = remember(logs) {
        (logs?.lines ?: emptyList())
            .mapIndexed { idx, raw -> parseServerLogLine(raw, idx) }
            .reversed()
    }
    val filtered = remember(parsed, query) {
        val trimmed = query.trim().lowercase()
        if (trimmed.isEmpty()) parsed
        else parsed.filter { line ->
            line.message.lowercase().contains(trimmed)
                || line.source?.lowercase()?.contains(trimmed) == true
                || line.level?.lowercase()?.contains(trimmed) == true
        }
    }

    Scaffold(
        topBar = {
            // Root screen: the shared navbar (online dot + account menu).
            HalogenNavbar(
                core = core,
                title = "Server logs",
                leading = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                        }
                    }
                },
                actions = {
                    IconButton(onClick = { scope.launch { load() } }, enabled = !loading) {
                        if (loading) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                        } else {
                            Icon(halogenIcon("arrow.clockwise"), contentDescription = "Refresh")
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
            val data = logs
            when {
                failure != null -> AdminUnavailableView(
                    icon = "wifi.exclamationmark",
                    title = "Couldn't load logs",
                    description = failure,
                    monospaced = true,
                )

                data == null -> Box(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
                    contentAlignment = Alignment.Center,
                ) { CircularProgressIndicator() }

                data.lines.isEmpty() -> AdminUnavailableView(
                    icon = "doc.text",
                    title = "No log lines",
                    description = if (data.path == null)
                        "This server has no log file configured (embedded servers log to the app console)."
                    else "The log file is empty.",
                )

                else -> LazyColumn(Modifier.fillMaxSize()) {
                    item(key = "search") {
                        OutlinedTextField(
                            value = query,
                            onValueChange = { query = it },
                            placeholder = { Text("Search logs…") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(
                                capitalization = KeyboardCapitalization.None,
                                autoCorrectEnabled = false,
                                keyboardType = KeyboardType.Text,
                            ),
                            modifier = Modifier.fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                    }
                    item(key = "header") {
                        Text(
                            "${filtered.size} of ${parsed.size} lines • newest first",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                    }
                    if (filtered.isEmpty()) {
                        item(key = "empty") {
                            Text(
                                "No lines match your search.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                        }
                    } else {
                        items(filtered, key = { it.id }) { line -> ServerLogRow(line) }
                    }
                }
            }
        }
    }
}

/// One parsed tracing line. Raw lines are ANSI-coded
/// `<ISO8601> LEVEL <file>:<line>: <message>`; anything that doesn't
/// parse renders whole as the message (never drop a line).
internal data class ServerLogLine(
    val id: Int,
    val time: String?,
    val level: String?,
    val source: String?,
    val message: String,
)

private val ansiCodes = Regex("\u001B\\[[0-9;]*m")
private val logLevels = setOf("TRACE", "DEBUG", "INFO", "WARN", "ERROR")

/// `2026-07-29T05:21:48.329717Z  INFO crates/server/src/x.rs:146: msg`
/// (with ANSI color codes) → level/time/source/message.
internal fun parseServerLogLine(raw: String, id: Int): ServerLogLine {
    val clean = raw.replace(ansiCodes, "")
    val parts = clean.split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (parts.size < 3 || !parts[0].contains("T") || parts[1] !in logLevels) {
        return ServerLogLine(id, time = null, level = null, source = null, message = clean)
    }
    // hh:mm:ss out of the ISO timestamp.
    val time = parts[0].split("T").lastOrNull()?.take(8)
    val level = parts[1]
    // Source is `path/file.rs:line:` — shorten to its last two segments.
    var source: String? = null
    var messageStart = 2
    if (parts.size > 3 && parts[2].endsWith(":")) {
        val full = parts[2].dropLast(1)
        source = full.split("/").takeLast(2).joinToString("/")
        messageStart = 3
    }
    val message = parts.drop(messageStart).joinToString(" ")
    return ServerLogLine(id, time = time, level = level, source = source, message = message)
}

@Composable
private fun ServerLogRow(line: ServerLogLine) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 3.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            line.level?.let { level ->
                Text(
                    level,
                    style = MaterialTheme.typography.labelSmall.copy(
                        fontWeight = FontWeight.Bold),
                    color = levelColor(level),
                )
            }
            line.time?.let { time ->
                Text(
                    time,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
            line.source?.let { source ->
                Text(
                    source,
                    style = MaterialTheme.typography.labelSmall.copy(
                        fontFamily = FontFamily.Monospace),
                    color = MaterialTheme.colorScheme.outline,
                    maxLines = 1,
                )
            }
        }
        Text(
            line.message,
            style = MaterialTheme.typography.labelSmall.copy(
                fontFamily = FontFamily.Monospace),
        )
    }
}

@Composable
private fun levelColor(level: String): Color = when (level) {
    "ERROR" -> MaterialTheme.colorScheme.error
    "WARN" -> halogenExtras.warning
    "INFO" -> MaterialTheme.colorScheme.primary
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}
