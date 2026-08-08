package org.fgsec.halogen.features.admin

import android.content.Intent
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
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import java.io.File
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.HalogenCore

/// This device's captured app events (the in-app DeviceLog ring) — the native
/// `/logs/device`: live capture toggle + threshold, search, share-sheet export,
/// and a clear that also wipes persisted storage.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeviceLogsView(core: HalogenCore, onBack: (() -> Unit)? = null) {
    val log = DeviceLog.shared
    var query by remember { mutableStateOf("") }
    val context = LocalContext.current

    val filtered = remember(log.entries, query) {
        val all = log.entries.reversed()
        val q = query.trim().lowercase()
        if (q.isEmpty()) all
        // Message OR source tag (web: search matches msg + target).
        else all.filter { entry ->
            entry.message.lowercase().contains(q)
                || entry.source?.lowercase()?.contains(q) == true
        }
    }

    fun export() {
        try {
            val file = File(context.cacheDir, "halogen-device-logs.txt")
            file.writeText(log.exportText())
            val send = Intent(Intent.ACTION_SEND).apply { type = "text/plain" }
            try {
                val uri = FileProvider.getUriForFile(
                    context, "${context.packageName}.fileprovider", file)
                send.putExtra(Intent.EXTRA_STREAM, uri)
                send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            } catch (e: Exception) {
                // No FileProvider configured — fall back to inline text.
                DeviceLog.warn(
                    "DeviceLogsView: FileProvider export failed — ${e::class.simpleName}: ${e.message}")
                send.putExtra(Intent.EXTRA_TEXT, log.exportText())
            }
            context.startActivity(Intent.createChooser(send, "halogen-device-logs.txt"))
        } catch (e: Exception) {
            ToastCenter.error("Export failed: $e")
        }
    }

    Scaffold(
        topBar = {
            // Root screen: the shared navbar (online dot + account menu).
            HalogenNavbar(
                core = core,
                title = "Device logs",
                leading = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                        }
                    }
                },
                actions = {
                    TextButton(
                        onClick = { log.clear() },
                        enabled = log.entries.isNotEmpty(),
                    ) { Text("Clear") }
                    IconButton(onClick = { export() }, enabled = log.entries.isNotEmpty()) {
                        Icon(halogenIcon("square.and.arrow.up"), contentDescription = "Export")
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding)) {
            item(key = "capture-header") { SectionHeader("Capture") }
            item(key = "capture-toggle") {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("Enable device logs", Modifier.weight(1f))
                    Switch(checked = log.enabled, onCheckedChange = { log.enabled = it })
                }
            }
            item(key = "capture-level") { LevelPickerRow(log) }

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

            item(key = "lines-header") {
                SectionHeader(
                    if (log.enabled) "Capturing • ${log.entries.size} lines"
                    else "Capture disabled"
                )
            }
            if (filtered.isEmpty()) {
                item(key = "empty") {
                    Text(
                        if (query.trim().isEmpty()) {
                            if (log.enabled) "No logs captured yet."
                            else "Capture disabled — enable device logs to record events."
                        } else "No logs match your search.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            } else {
                items(filtered, key = { it.id }) { entry -> DeviceLogRow(entry) }
            }
        }
    }
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

/// The level-threshold picker (iOS Picker parity): current label opens a
/// menu of DeviceLog.Level labels; selection applies live.
@Composable
private fun LevelPickerRow(log: DeviceLog) {
    var open by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth()
            .clickable { open = true }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Log level", Modifier.weight(1f))
        Box {
            Text(
                log.threshold.label,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                for (level in DeviceLog.Level.entries) {
                    DropdownMenuItem(
                        text = { Text(level.label) },
                        trailingIcon = {
                            if (level == log.threshold) {
                                Icon(halogenIcon("checkmark"), contentDescription = null)
                            }
                        },
                        onClick = {
                            log.threshold = level
                            open = false
                        },
                    )
                }
            }
        }
    }
}

/// iOS `.dateTime.hour().minute().second()` shape ("05:21:48").
private val entryTimeFormatter =
    DateTimeFormatter.ofPattern("HH:mm:ss").withZone(ZoneId.systemDefault())

@Composable
private fun DeviceLogRow(entry: DeviceLog.Entry) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 3.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                entry.level.token.uppercase(),
                style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold),
                color = levelColor(entry.level),
            )
            Text(
                entryTimeFormatter.format(Instant.ofEpochMilli(entry.at)),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.outline,
            )
            // Rust tracing target — distinguishes embedded-server lines from
            // app lines (web: the target column on /logs/device).
            entry.source?.let { source ->
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
            entry.message,
            style = MaterialTheme.typography.labelSmall.copy(
                fontFamily = FontFamily.Monospace),
        )
    }
}

@Composable
private fun levelColor(level: DeviceLog.Level): Color = when (level) {
    DeviceLog.Level.Info -> MaterialTheme.colorScheme.onSurfaceVariant
    DeviceLog.Level.Warn -> halogenExtras.warning
    DeviceLog.Level.Error -> MaterialTheme.colorScheme.error
}
