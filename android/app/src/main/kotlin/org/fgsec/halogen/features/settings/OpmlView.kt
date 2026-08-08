package org.fgsec.halogen.features.settings

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.unit.dp
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// OPML import/export (admin — the web's Settings → Podcasts). Export saves
/// the server's OPML via SAF; import reads a picked .opml/.xml and posts it.
@Composable
fun OpmlView(core: HalogenCore, onBack: () -> Unit) {
    var exported by remember { mutableStateOf<String?>(null) }
    var result by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    // SAF stand-in for the iOS share sheet (ANDROID_NATIVE §6).
    val saveLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("text/xml"),
    ) { uri: Uri? ->
        val opml = exported
        if (uri != null && opml != null) {
            scope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        context.contentResolver.openOutputStream(uri)?.use {
                            it.write(opml.toByteArray(Charsets.UTF_8))
                        } ?: throw IOException("Could not open the picked location")
                    }
                    ToastCenter.success("Saved halogen.opml")
                } catch (e: Exception) {
                    error = FriendlyError.message(e)
                }
            }
        }
    }

    suspend fun importOpml(uri: Uri) {
        busy = true
        try {
            val opml = withContext(Dispatchers.IO) {
                context.contentResolver.openInputStream(uri)?.use {
                    String(it.readBytes(), Charsets.UTF_8)
                } ?: throw IOException("Could not read file")
            }
            val outcome = core.opmlImport(opml)
            result = "${outcome.created} created · ${outcome.skipped} skipped · ${outcome.errors} errors"
            core.models?.podcasts?.refresh()
            error = null
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    val importLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        if (uri != null) scope.launch { importOpml(uri) }
    }

    suspend fun exportOpml() {
        busy = true
        try {
            exported = core.opmlExport()
            error = null
            saveLauncher.launch("halogen.opml")
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            busy = false
        }
    }

    SettingsScaffold(title = "OPML", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(header = "Export") {
                SettingsRow(
                    onClick = { scope.launch { exportOpml() } },
                    enabled = !busy,
                ) {
                    Icon(
                        halogenIcon("square.and.arrow.up"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text("Export subscriptions…", Modifier.weight(1f))
                }
                if (exported != null) {
                    SettingsRow(onClick = { saveLauncher.launch("halogen.opml") }) {
                        Icon(
                            halogenIcon("doc.badge.arrow.up"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text("Save halogen.opml", Modifier.weight(1f))
                    }
                }
            }

            SettingsGroup(header = "Import") {
                SettingsRow(
                    onClick = {
                        importLauncher.launch(
                            arrayOf(
                                "text/xml", "application/xml", "text/x-opml",
                                "application/octet-stream",
                            ))
                    },
                    enabled = !busy,
                ) {
                    Icon(
                        halogenIcon("square.and.arrow.down"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text("Import OPML file…", Modifier.weight(1f))
                }
            }

            result?.let {
                SettingsGroup(header = "Result") {
                    SettingsRow { Text(it) }
                }
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
