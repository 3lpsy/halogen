package org.fgsec.halogen.features.discover

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.DiscoverProviderInfo
import org.fgsec.halogen.wire.DiscoverResultItem

/// Discover: online podcast search across the server's providers. Bare
/// results by design (no artwork — the server never proxies images here);
/// subscribing creates the podcast and the next poll ingests episodes.
/// Online-only, like the web.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiscoverView(model: DiscoverModel, core: HalogenCore) {
    val scope = rememberCoroutineScope()
    var detail by remember { mutableStateOf<DiscoverResultItem?>(null) }

    LaunchedEffect(Unit) { model.loadProviders() }

    Column(Modifier.fillMaxSize()) {
        HalogenNavbar(core = core, title = "Discover")
        TextField(
            value = model.query,
            onValueChange = { model.query = it },
            placeholder = { Text("Podcast name") },
            leadingIcon = { Icon(halogenIcon("magnifyingglass"), contentDescription = null) },
            trailingIcon = {
                if (model.query.isNotEmpty()) {
                    IconButton(onClick = { model.query = "" }) {
                        Icon(halogenIcon("xmark.circle.fill"), contentDescription = "Clear")
                    }
                }
            },
            singleLine = true,
            shape = RoundedCornerShape(9.dp),
            colors = TextFieldDefaults.colors(
                focusedIndicatorColor = MaterialTheme.colorScheme.surfaceVariant,
                unfocusedIndicatorColor = MaterialTheme.colorScheme.surfaceVariant,
            ),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { scope.launch { model.search() } }),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
        StatusBar(model)
        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (model.results.isEmpty() && !model.searching) {
                DiscoverEmptyState(
                    title = if (model.query.isEmpty()) "Search podcasts" else "No results",
                    description =
                        if (model.query.isEmpty())
                            "Search the server's providers. Searches need at least 2 characters."
                        else "Nothing matched \"${model.query}\".",
                )
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(model.results, key = { it.id }) { item ->
                        DiscoverRow(
                            item = item,
                            providerLabel = model.providerLabel(item.provider),
                            subscribed = model.isSubscribed(item),
                            subscribe = { scope.launch { model.subscribe(item) } },
                            open = { detail = item },
                        )
                    }
                }
            }
            if (model.searching) {
                CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
        }
    }

    detail?.let { item ->
        // Per-result detail (the web's /discover/:id): full description,
        // provider, feed URL, subscribe.
        ModalBottomSheet(onDismissRequest = { detail = null }) {
            DiscoverDetailSheet(
                item = item,
                model = model,
                subscribe = {
                    scope.launch {
                        model.subscribe(item)
                        detail = null
                    }
                },
                close = { detail = null },
            )
        }
    }

    if (model.error != null) {
        AlertDialog(
            onDismissRequest = { model.clearError() },
            title = { Text("Search failed") },
            text = { Text(model.error ?: "") },
            confirmButton = { TextButton(onClick = { model.clearError() }) { Text("OK") } },
        )
    }
}

/// Offline banner / provider-fetch retry / provider toggle chips — the web
/// Discover page's pre-results block.
@Composable
private fun StatusBar(model: DiscoverModel) {
    val hasContent = model.isOffline ||
        (model.providerError && model.providers.isEmpty()) ||
        model.providers.isNotEmpty()
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp)
            .padding(vertical = if (hasContent) 8.dp else 0.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (model.isOffline) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    halogenIcon("wifi.slash"),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    "Discover needs an internet connection.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 6.dp),
                )
            }
        } else if (model.providerError && model.providers.isEmpty()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Couldn't load the search providers.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { model.retryProviders() }) {
                    Text("Retry", style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        if (model.providers.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (info in model.providers) {
                    ProviderChip(info, model)
                }
            }
        }
    }
}

/// One provider toggle chip: on = searched, off/unavailable = skipped.
@Composable
private fun ProviderChip(info: DiscoverProviderInfo, model: DiscoverModel) {
    val on = info.available && model.isEnabled(info.id)
    Surface(
        onClick = { model.toggleProvider(info.id) },
        enabled = info.available,
        shape = RoundedCornerShape(50),
        color = if (on) MaterialTheme.colorScheme.primary.copy(alpha = 0.2f)
        else MaterialTheme.colorScheme.surfaceVariant,
        contentColor = if (on) MaterialTheme.colorScheme.primary
        else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.alpha(if (info.available) 1f else 0.5f),
    ) {
        Text(
            info.label,
            style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Medium),
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
        )
    }
}

/// One bare search result row: text only, subscribe button at the trailing
/// edge; tapping elsewhere opens the detail sheet.
@Composable
private fun DiscoverRow(
    item: DiscoverResultItem,
    providerLabel: String,
    subscribed: Boolean,
    subscribe: () -> Unit,
    open: () -> Unit,
) {
    Surface(onClick = open, color = MaterialTheme.colorScheme.surface) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(
                    item.title,
                    style = MaterialTheme.typography.bodyMedium
                        .copy(fontWeight = FontWeight.Medium),
                )
                val author = item.author
                if (!author.isNullOrEmpty()) {
                    Text(
                        author,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val description = item.description
                if (!description.isNullOrEmpty()) {
                    Text(
                        description,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    providerLabel,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
            IconButton(onClick = subscribe, enabled = !subscribed) {
                Icon(
                    halogenIcon(if (subscribed) "checkmark.circle.fill" else "plus.circle"),
                    contentDescription = if (subscribed) "Subscribed" else "Subscribe",
                    tint = if (subscribed) halogenExtras.success
                    else MaterialTheme.colorScheme.primary,
                )
            }
        }
    }
}

/// The detail sheet body: Close / Subscribe bar, then the full result.
@Composable
private fun DiscoverDetailSheet(
    item: DiscoverResultItem,
    model: DiscoverModel,
    subscribe: () -> Unit,
    close: () -> Unit,
) {
    val subscribed = model.isSubscribed(item)
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(onClick = close) { Text("Close") }
            Spacer(Modifier.weight(1f))
            Text("Podcast", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.weight(1f))
            TextButton(onClick = subscribe, enabled = !subscribed) {
                Text(if (subscribed) "Subscribed" else "Subscribe")
            }
        }
        Column(
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                item.title,
                style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold),
            )
            val author = item.author
            if (!author.isNullOrEmpty()) {
                Text(
                    author,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(
                model.providerLabel(item.provider),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.outline,
            )
            SelectionContainer {
                Text(
                    item.feed_url,
                    style = MaterialTheme.typography.labelSmall
                        .copy(fontFamily = FontFamily.Monospace),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            val description = item.description
            if (!description.isNullOrEmpty()) {
                HorizontalDivider()
                Text(description, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

/// iOS ContentUnavailableView parity.
@Composable
private fun DiscoverEmptyState(title: String, description: String) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon("magnifyingglass"),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(44.dp),
        )
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(
            description,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
