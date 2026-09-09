package org.fgsec.halogen.features.podcasts

import org.fgsec.halogen.core.ensureQueued
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.OutboxOp

/// Subscribe by feed URL (the web's `/podcasts/create`). The next poll
/// ingests episodes; Discover is the search-first alternative.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PodcastCreateSheet(
    core: HalogenCore,
    onDismiss: () -> Unit,
    onCreated: suspend () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var title by remember { mutableStateOf("") }
    var feedUrl by remember { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }

    // Title optional (falls back to the feed URL until the first ingest heals
    // it — web rule); the URL must at least parse as http(s), or the durable
    // op dead-letters silently after the user has moved on.
    val feedUrlValid = feedUrl.trim().toHttpUrlOrNull() != null

    fun save() {
        if (saving) return
        scope.launch {
            saving = true
            try {
                // Durable subscribe (web: OutboxOp::Subscribe) — queues
                // offline; the podcast appears once the op drains and the
                // library refreshes. An empty title rides as null; the drain
                // falls back to the feed URL.
                val trimmedTitle = title.trim()
                if (!core.ensureQueued(
                    OutboxOp.Kind.Subscribe(
                        feedUrl = feedUrl.trim(),
                        title = trimmedTitle.ifEmpty { null },
                        description = null,
                    )
                )) return@launch
                onCreated()
            } finally {
                saving = false
            }
            onDismiss()
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .navigationBarsPadding()
                .padding(bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onDismiss) { Text("Cancel") }
                Spacer(Modifier.weight(1f))
                Text("Add podcast", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.weight(1f))
                TextButton(
                    onClick = { save() },
                    enabled = !saving && feedUrlValid,
                    modifier = Modifier.testTag("podcast-create-submit"),
                ) {
                    Text("Add")
                }
            }
            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                label = { Text("Title") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = feedUrl,
                onValueChange = { feedUrl = it },
                label = { Text("Feed URL (https://…)") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Uri,
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}
