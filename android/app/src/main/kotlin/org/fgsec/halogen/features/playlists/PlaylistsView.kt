package org.fgsec.halogen.features.playlists

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DragHandle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.AppRoute
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LocalNavigator
import org.fgsec.halogen.components.RowSwipeAction
import org.fgsec.halogen.components.SortSearchBar
import org.fgsec.halogen.components.SwipeActionsRow
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.components.rememberReorderableListState
import org.fgsec.halogen.components.reorderHandle
import org.fgsec.halogen.components.reorderableItem
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistData
import org.fgsec.halogen.wire.PlaylistReorderField

/// The Playlists tab: every playlist (the queue badged as such), episode
/// counts from the EpisodeIds include, create/delete, drill into detail.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlaylistsView(model: PlaylistsModel, core: HalogenCore) {
    var editing by remember { mutableStateOf(false) }
    var showCreate by remember { mutableStateOf(false) }
    var renameTarget by remember { mutableStateOf<PlaylistData?>(null) }
    var deleteTarget by remember { mutableStateOf<PlaylistData?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) { model.load() }

    Column(Modifier.fillMaxSize()) {
        HalogenNavbar(core = core, title = "Playlists", leading = {
            TextButton(onClick = { editing = !editing }) {
                Text(if (editing) "Done" else "Edit")
            }
        }) {
            IconButton(
                onClick = { showCreate = true },
                modifier = Modifier.testTag("playlist-add"),
            ) {
                Icon(halogenIcon("plus"), contentDescription = "New playlist")
            }
        }
        SortSearchBar(
            search = model.query.search,
            onSearchChange = { model.query = model.query.copy(search = it) },
            field = model.query.field,
            onFieldChange = { model.query = model.query.copy(field = it) },
            direction = model.query.direction,
            onDirectionChange = { model.query = model.query.copy(direction = it) },
            fields = PlaylistSortField.entries.map { it to it.label },
            placeholder = "Search playlists",
        )
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    model.refresh()
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize(),
        ) {
            val error = model.error
            when {
                error != null ->
                    LoadErrorView(title = "Couldn't load playlists", message = error) {
                        model.refresh()
                    }

                model.loaded && model.playlists.isEmpty() ->
                    EmptyStateView(
                        systemImage = "music.note.list",
                        title = "No playlists yet",
                        description = "Create a playlist to organize episodes.",
                    )

                model.loaded && model.displayed.isEmpty() ->
                    EmptyStateView(
                        systemImage = "magnifyingglass",
                        title = "No results for \"${model.query.search.trim()}\"",
                        description = "Check the spelling or try a new search.",
                    )

                else -> PlaylistsList(
                    model = model, core = core, editing = editing,
                    onRename = { renameTarget = it },
                    onDelete = { deleteTarget = it },
                )
            }
        }
    }

    if (showCreate) {
        PlaylistCreateSheet(model = model, core = core, onDismiss = { showCreate = false })
    }
    renameTarget?.let { target ->
        PlaylistEditSheet(
            model = model, core = core, playlist = target,
            onDismiss = { renameTarget = null },
        )
    }
    deleteTarget?.let { target ->
        // Deleting is irreversible (server rows + files) — confirm first.
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = { Text("Delete playlist?") },
            text = { Text("Removes the playlist everywhere. Its episodes stay in the library.") },
            confirmButton = {
                TextButton(onClick = {
                    scope.launch { model.delete(target) }
                    deleteTarget = null
                }) {
                    Text("Delete ${target.name}", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteTarget = null }) { Text("Cancel") }
            },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PlaylistsList(
    model: PlaylistsModel,
    core: HalogenCore,
    editing: Boolean,
    onRename: (PlaylistData) -> Unit,
    onDelete: (PlaylistData) -> Unit,
) {
    val navigator = LocalNavigator.current
    val listState = rememberLazyListState()
    val reorder = rememberReorderableListState(listState) { from, to ->
        // Custom-asc + no search only (web rule) — offline-capable
        // optimistic reorder + a durable MovePlaylist op.
        if (model.reorderable) model.move(from, to)
    }
    val canReorder = editing && model.reorderable

    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
        itemsIndexed(model.displayed, key = { _, playlist -> playlist.id }) { index, playlist ->
            var menuOpen by remember { mutableStateOf(false) }
            Box(Modifier.reorderableItem(reorder, index)) {
                SwipeActionsRow(
                    trailing = listOf(
                        RowSwipeAction(label = "Delete", systemImage = "trash", destructive = true) {
                            onDelete(playlist)
                        }
                    ),
                ) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            // Long-press mirror of the ellipsis menu (same
                            // shared content — the two can't drift).
                            .combinedClickable(
                                onClick = { navigator?.push(AppRoute.Playlist(playlist.id)) },
                                onLongClick = { menuOpen = true },
                            )
                            .padding(horizontal = 16.dp, vertical = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PlaylistRow(playlist = playlist, modifier = Modifier.weight(1f))
                        Box {
                            IconButton(onClick = { menuOpen = true }) {
                                Icon(
                                    halogenIcon("ellipsis"),
                                    contentDescription = "Playlist actions",
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            PlaylistMenu(
                                expanded = menuOpen,
                                onDismiss = { menuOpen = false },
                                playlist = playlist,
                                model = model,
                                core = core,
                                onEdit = { onRename(playlist) },
                                onDelete = { onDelete(playlist) },
                            )
                        }
                        if (canReorder) {
                            Icon(
                                Icons.Rounded.DragHandle,
                                contentDescription = "Reorder",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.reorderHandle(reorder, playlist.id),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun PlaylistRow(playlist: PlaylistData, modifier: Modifier = Modifier) {
    Row(
        modifier,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            halogenIcon(if (playlist.is_default) "list.bullet.circle.fill" else "music.note.list"),
            contentDescription = null,
            tint = if (playlist.is_default) MaterialTheme.colorScheme.primary
            else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(28.dp),
        )
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(playlist.name, style = MaterialTheme.typography.titleSmall)
                if (playlist.is_default) {
                    Text(
                        "Queue",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier
                            .background(
                                MaterialTheme.colorScheme.primary.copy(alpha = 0.15f),
                                RoundedCornerShape(50),
                            )
                            .padding(horizontal = 6.dp, vertical = 2.dp),
                    )
                }
            }
            playlist.episode_ids?.let { ids ->
                Text(
                    "${ids.size} episodes",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/// Per-playlist quick actions: edit, smart reorder, make-queue, delete —
/// shared by the ellipsis button and the long-press so both always agree.
@Composable
fun PlaylistMenu(
    expanded: Boolean,
    onDismiss: () -> Unit,
    playlist: PlaylistData,
    model: PlaylistsModel,
    core: HalogenCore,
    onEdit: () -> Unit,
    onDelete: () -> Unit,
) {
    // Compose menus have no native submenu — "Reorder by" flips to a second
    // page listing field × direction inside the same dropdown.
    var reorderPage by remember(expanded) { mutableStateOf(false) }
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        if (!reorderPage) {
            DropdownMenuItem(
                text = { Text("Edit") },
                leadingIcon = { Icon(halogenIcon("pencil"), contentDescription = null) },
                onClick = {
                    onDismiss()
                    onEdit()
                },
            )
            DropdownMenuItem(
                text = { Text("Reorder by") },
                leadingIcon = { Icon(halogenIcon("arrow.up.arrow.down"), contentDescription = null) },
                trailingIcon = { Icon(halogenIcon("chevron.right"), contentDescription = null) },
                onClick = { reorderPage = true },
            )
            if (!playlist.is_default) {
                DropdownMenuItem(
                    text = { Text("Make this the queue") },
                    leadingIcon = {
                        Icon(halogenIcon("list.bullet.circle"), contentDescription = null)
                    },
                    onClick = {
                        onDismiss()
                        makeQueue(core, model, playlist)
                    },
                )
            }
            HorizontalDivider()
            DropdownMenuItem(
                text = { Text("Delete", color = MaterialTheme.colorScheme.error) },
                leadingIcon = {
                    Icon(
                        halogenIcon("trash"), contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                    )
                },
                onClick = {
                    onDismiss()
                    onDelete()
                },
            )
        } else {
            for ((direction, label) in listOf(
                OrderDirection.Asc to "Ascending",
                OrderDirection.Desc to "Descending",
            )) {
                DropdownMenuItem(
                    text = {
                        Text(
                            label,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    },
                    onClick = {},
                    enabled = false,
                )
                for (field in PlaylistReorderField.entries) {
                    DropdownMenuItem(
                        text = { Text(field.name) },
                        onClick = {
                            onDismiss()
                            // Durable (web: ReorderPlaylist op) — queues
                            // offline instead of silently dropping the action.
                            core.scope.launch {
                                core.outbox?.enqueue(
                                    OutboxOp.Kind.ReorderPlaylist(
                                        playlistId = playlist.id,
                                        field = field, direction = direction))
                                model.refresh()
                            }
                        },
                    )
                }
            }
        }
    }
}

private fun makeQueue(core: HalogenCore, model: PlaylistsModel, playlist: PlaylistData) {
    core.scope.launch {
        if (core.isOffline) {
            // Offline: optimistic flip + queued UpdatePlaylist (web rule —
            // edits queue offline, go direct online).
            model.markDefaultLocally(playlist.id)
            core.outbox?.enqueue(
                OutboxOp.Kind.UpdatePlaylist(playlistId = playlist.id, isDefault = true))
        } else {
            try {
                core.makeQueuePlaylist(playlist.id)
            } catch (e: Exception) {
                ToastCenter.error(
                    "Couldn't make \"${playlist.name}\" the queue — ${FriendlyError.message(e)}")
            }
            model.refresh()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlaylistCreateSheet(
    model: PlaylistsModel,
    core: HalogenCore,
    onDismiss: () -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var makeDefault by remember { mutableStateOf(false) }
    var deleteServerFile by remember { mutableStateOf(false) }
    var deleteClientFile by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    // No queue yet → the first playlist MUST become it (web: force_default;
    // the server enforces this too).
    val forceDefault = core.models?.queue?.loaded == true && core.models?.queue?.queue == null
    LaunchedEffect(forceDefault) { if (forceDefault) makeDefault = true }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SheetHeader(
                title = "New Playlist",
                confirmLabel = "Create",
                confirmEnabled = name.trim().isNotEmpty(),
                onCancel = onDismiss,
                onConfirm = {
                    scope.launch {
                        try {
                            val desc = description.trim()
                            model.create(
                                name = name.trim(),
                                description = desc.ifEmpty { null },
                                isDefault = makeDefault || forceDefault,
                                deleteServerFile = deleteServerFile,
                                deleteClientFile = deleteClientFile,
                            )
                            onDismiss()
                        } catch (e: Exception) {
                            error = FriendlyError.message(e)
                        }
                    }
                },
            )
            OutlinedTextField(
                value = name, onValueChange = { name = it },
                label = { Text("Name") }, singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = description, onValueChange = { description = it },
                label = { Text("Description (optional)") },
                modifier = Modifier.fillMaxWidth(),
            )
            ToggleRow(
                label = "Make this the queue (default)",
                checked = makeDefault, enabled = !forceDefault,
            ) { makeDefault = it }
            if (forceDefault) {
                Footnote("You don't have a queue yet — this playlist will become it.")
            }
            ToggleRow(label = "Delete server download on remove", checked = deleteServerFile) {
                deleteServerFile = it
            }
            ToggleRow(label = "Delete device download on remove", checked = deleteClientFile) {
                deleteClientFile = it
            }
            Footnote(
                "Removing an episode from this playlist also deletes its downloaded file on the server (unless another playlist still has it) and/or on the removing device."
            )
            error?.let {
                Text(it, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

/// Edit a playlist (`PUT /playlists/{id}`) — the web form's full field set:
/// name, description, make-default, and the two delete-on-remove flags.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlaylistEditSheet(
    model: PlaylistsModel,
    core: HalogenCore,
    playlist: PlaylistData,
    onDismiss: () -> Unit,
) {
    var name by remember(playlist.id) { mutableStateOf(playlist.name) }
    var description by remember(playlist.id) { mutableStateOf(playlist.description ?: "") }
    var makeDefault by remember(playlist.id) { mutableStateOf(playlist.is_default) }
    var deleteServerFile by remember(playlist.id) {
        mutableStateOf(playlist.on_remove_delete_file_server ?: false)
    }
    var deleteClientFile by remember(playlist.id) {
        mutableStateOf(playlist.on_remove_delete_file_client ?: false)
    }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    suspend fun save() {
        val trimmed = name.trim()
        val desc = description.trim()
        // The web edit sends the FULL prefilled set, so unchanged fields
        // round-trip their current values (null would mean "leave unchanged"
        // and make blanked fields unreachable).
        if (core.isOffline) {
            // Offline: optimistic + queued UpdatePlaylist (online goes
            // direct so the form can show server errors — web rule).
            model.updateLocally(
                id = playlist.id, name = trimmed,
                description = desc.ifEmpty { null },
                isDefault = makeDefault,
                deleteServerFile = deleteServerFile,
                deleteClientFile = deleteClientFile,
            )
            core.outbox?.enqueue(
                OutboxOp.Kind.UpdatePlaylist(
                    playlistId = playlist.id, name = trimmed, isDefault = makeDefault,
                    description = desc.ifEmpty { null },
                    deleteServerFile = deleteServerFile,
                    deleteClientFile = deleteClientFile,
                ))
            onDismiss()
            return
        }
        try {
            core.updatePlaylist(
                id = playlist.id, name = trimmed,
                description = desc.ifEmpty { null },
                isDefault = makeDefault,
                deleteServerFile = deleteServerFile,
                deleteClientFile = deleteClientFile,
            )
            model.refresh()
            onDismiss()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SheetHeader(
                title = "Edit playlist",
                confirmLabel = "Save",
                confirmEnabled = name.trim().isNotEmpty(),
                onCancel = onDismiss,
                onConfirm = { scope.launch { save() } },
            )
            OutlinedTextField(
                value = name, onValueChange = { name = it },
                label = { Text("Name") }, singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = description, onValueChange = { description = it },
                label = { Text("Description (optional)") },
                modifier = Modifier.fillMaxWidth(),
            )
            ToggleRow(
                label = "Make this the queue (default)",
                checked = makeDefault, enabled = !playlist.is_default,
            ) { makeDefault = it }
            ToggleRow(label = "Delete server download on remove", checked = deleteServerFile) {
                deleteServerFile = it
            }
            ToggleRow(label = "Delete device download on remove", checked = deleteClientFile) {
                deleteClientFile = it
            }
            Footnote(
                "Removing an episode from this playlist also deletes its downloaded file on the server (unless another playlist still has it) and/or on the removing device."
            )
            error?.let {
                Text(it, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

@Composable
private fun SheetHeader(
    title: String,
    confirmLabel: String,
    confirmEnabled: Boolean,
    onCancel: () -> Unit,
    onConfirm: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TextButton(onClick = onCancel) { Text("Cancel") }
        Text(title, style = MaterialTheme.typography.titleMedium)
        Button(onClick = onConfirm, enabled = confirmEnabled) { Text(confirmLabel) }
    }
}

@Composable
private fun ToggleRow(
    label: String,
    checked: Boolean,
    enabled: Boolean = true,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onCheckedChange, enabled = enabled)
    }
}

@Composable
private fun Footnote(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
