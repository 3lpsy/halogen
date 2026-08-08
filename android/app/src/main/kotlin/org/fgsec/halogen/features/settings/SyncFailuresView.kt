package org.fgsec.halogen.features.settings

import android.text.format.DateUtils
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.SyncFailures

/// Changes the server permanently rejected (dead-lettered sync ops). The
/// optimistic UI reverts on a later refresh; this page explains what and why.
@Composable
fun SyncFailuresView(failures: SyncFailures, onBack: () -> Unit) {
    SettingsScaffold(
        title = "Sync failures",
        onBack = onBack,
        actions = {
            TextButton(
                onClick = { failures.clear() },
                enabled = failures.failures.isNotEmpty(),
            ) { Text("Clear") }
        },
    ) { padding ->
        if (failures.failures.isEmpty()) {
            Column(
                Modifier.fillMaxSize().padding(padding).padding(32.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Icon(
                    halogenIcon("checkmark.circle"),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(48.dp),
                )
                Text("No sync failures", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Changes the server rejects appear here.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
        } else {
            Column(
                Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
            ) {
                SettingsGroup(
                    footer = "These changes were rejected by the server and undone. Redo one if you still want it.",
                ) {
                    for (failure in failures.failures.reversed()) {
                        Column(
                            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                            verticalArrangement = Arrangement.spacedBy(2.dp),
                        ) {
                            Text(
                                failure.summary,
                                style = MaterialTheme.typography.bodyMedium.copy(
                                    fontWeight = FontWeight.Medium),
                            )
                            Text(
                                failure.reason,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                DateUtils.getRelativeTimeSpanString(
                                    failure.atInstant.toEpochMilli()).toString(),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }
}
