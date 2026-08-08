package org.fgsec.halogen.features.settings

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// Database export/import (admin): a gzipped, scrubbed snapshot out via SAF;
/// merge an export back in via the file picker.
@Composable
fun DbTransferView(core: HalogenCore, onBack: () -> Unit) {
    // Fetched export waiting to be saved: filename → gzip bytes.
    var export by remember { mutableStateOf<Pair<String, ByteArray>?>(null) }
    // A picked file waits for an explicit confirm — appending a whole
    // library shouldn't be one mis-tap away (web: staged_import).
    var staged by remember { mutableStateOf<Pair<String, ByteArray>?>(null) }
    var result by remember { mutableStateOf<String?>(null) }
    // Embedded-only caveat after an import created users (see importDb).
    var alignmentNote by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    val saveLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/gzip"),
    ) { uri: Uri? ->
        val pending = export
        if (uri != null && pending != null) {
            scope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        context.contentResolver.openOutputStream(uri)?.use {
                            it.write(pending.second)
                        } ?: throw IOException("Could not open the picked location")
                    }
                    ToastCenter.success("Saved ${pending.first}")
                } catch (e: Exception) {
                    error = FriendlyError.message(e)
                }
            }
        }
    }

    /// Read the picked file immediately and stage it for the explicit
    /// "Import now" confirm.
    fun stage(uri: Uri) {
        scope.launch {
            try {
                val (name, data) = withContext(Dispatchers.IO) {
                    val bytes = context.contentResolver.openInputStream(uri)?.use {
                        it.readBytes()
                    } ?: throw IOException("Could not read file")
                    displayName(context, uri) to bytes
                }
                staged = name to data
                error = null
            } catch (e: Exception) {
                error = "Could not read file: $e"
            }
        }
    }

    val pickLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        if (uri != null) stage(uri)
    }

    suspend fun exportDb() {
        busy = true
        try {
            val (data, filename) = core.dbExport()
            export = filename to data
            error = null
            saveLauncher.launch(filename)
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    suspend fun importDb(data: ByteArray) {
        busy = true
        try {
            val summary = core.dbImport(data)
            // The web's composed success toast, verbatim shape.
            result = "Import merged: ${summary.users_merged} user(s) matched, " +
                "${summary.users_created} created; +${summary.podcasts_created} podcast(s), " +
                "+${summary.episodes_created} episode(s), +${summary.playlists_created} playlist(s)"
            // Users the import created got random passwords — rotate them
            // into the silent-login secrets so switching to them just works
            // (web: align_imported_users via recover_user). Warn only for
            // whatever couldn't be aligned.
            alignmentNote = if (core.isEmbeddedAccount && summary.created_usernames.isNotEmpty()) {
                val failed = core.alignImportedUsers(summary.created_usernames)
                if (failed.isEmpty()) null
                else "Couldn't set up sign-in for imported user(s) " +
                    "${failed.joinToString(", ")} — they keep their random " +
                    "passwords and can't be signed in from this device."
            } else null
            staged = null
            error = null
            core.models?.podcasts?.refresh()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    SettingsScaffold(title = "Database", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(
                header = "Export",
                footer = "A gzipped, scrubbed snapshot — no password hashes, download flags, or history.",
            ) {
                SettingsRow(
                    onClick = { scope.launch { exportDb() } },
                    enabled = !busy,
                ) {
                    Icon(
                        halogenIcon("square.and.arrow.up"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text("Export database…", Modifier.weight(1f))
                }
                export?.let { (filename, _) ->
                    SettingsRow(onClick = { saveLauncher.launch(filename) }) {
                        Icon(
                            halogenIcon("doc.badge.arrow.up"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            filename,
                            Modifier.weight(1f),
                            maxLines = 1,
                            overflow = TextOverflow.MiddleEllipsis,
                        )
                    }
                }
            }

            SettingsGroup(
                header = "Import",
                footer = "Merges another server's export into this library: matching usernames merge, new users are created, nothing is replaced.",
            ) {
                val stagedFile = staged
                if (stagedFile != null) {
                    SettingsRow {
                        Text(
                            stagedFile.first,
                            style = MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace),
                            maxLines = 1,
                            overflow = TextOverflow.MiddleEllipsis,
                        )
                    }
                    SettingsRow(
                        // App scope: a successful import re-logs-in and can
                        // remount the tree, cancelling a composition scope.
                        onClick = { core.scope.launch { importDb(stagedFile.second) } },
                        enabled = !busy,
                    ) {
                        Icon(
                            halogenIcon("square.and.arrow.down"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text("Import now", Modifier.weight(1f))
                    }
                    SettingsRow(onClick = { staged = null }, enabled = !busy) {
                        Text("Cancel")
                    }
                } else {
                    SettingsRow(
                        onClick = {
                            pickLauncher.launch(
                                arrayOf(
                                    "application/gzip", "application/x-gzip",
                                    "application/octet-stream",
                                ))
                        },
                        enabled = !busy,
                    ) {
                        Icon(
                            halogenIcon("doc.badge.plus"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text("Choose export file…", Modifier.weight(1f))
                    }
                }
            }

            if (busy) {
                SettingsRow {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                    )
                    Text("Working…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            result?.let {
                SettingsGroup(header = "Result") {
                    SettingsRow { Text(it) }
                }
            }
            alignmentNote?.let {
                FootnoteText(
                    it,
                    color = MaterialTheme.colorScheme.tertiary,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            error?.let {
                FootnoteText(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
        }
    }
}

/// The picked document's display name (SAF URIs don't carry a path).
private fun displayName(context: Context, uri: Uri): String {
    context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
        val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
        if (index >= 0 && cursor.moveToFirst()) return cursor.getString(index)
    }
    return uri.lastPathSegment ?: "export"
}
