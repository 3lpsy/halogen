package org.fgsec.halogen.features.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DragHandle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.JsonKeyValueView
import org.fgsec.halogen.components.OnlineDot
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.BuiltinNav
import org.fgsec.halogen.core.ClientPrefs
import org.fgsec.halogen.core.ConnectionMonitor
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.NavModel
import org.fgsec.halogen.features.admin.ConfigOverridesView
import org.fgsec.halogen.features.admin.ServerErrorsView
import org.fgsec.halogen.storage.AccountContext

/// The Settings tab's in-module destinations (iOS NavigationLink pushes).
private enum class SettingsRoute {
    SyncFailures, Accounts, UsernameEdit, ConfigureDock, ConfigureSwipes,
    ChangePassword, ServerErrors, ViewConfig, ConfigOverrides, Opml,
    DbTransfer, LocalData,
}

/// Settings: account + server info, per-account preferences, dock/swipe
/// configuration, admin server surfaces, local data, sign out.
@Composable
fun SettingsView(core: HalogenCore, nav: NavModel) {
    var route by remember { mutableStateOf<SettingsRoute?>(null) }
    val back: () -> Unit = { route = null }

    when (route) {
        null -> SettingsRoot(core, nav, push = { route = it })
        SettingsRoute.SyncFailures -> {
            val failures = core.syncFailures
            if (failures != null) SyncFailuresView(failures, back)
            else SettingsRoot(core, nav, push = { route = it })
        }
        SettingsRoute.Accounts -> AccountsView(core, back)
        SettingsRoute.UsernameEdit -> UsernameEditView(core, back)
        SettingsRoute.ConfigureDock -> ConfigureDockView(nav, core.isAdmin, back)
        SettingsRoute.ConfigureSwipes -> {
            val swipes = core.models?.swipes
            if (swipes != null) ConfigureSwipesView(swipes, back)
            else SettingsRoot(core, nav, push = { route = it })
        }
        SettingsRoute.ChangePassword -> ChangePasswordView(core, back)
        SettingsRoute.ServerErrors -> ServerErrorsView(core, back)
        SettingsRoute.ViewConfig -> {
            BackHandler(onBack = back)
            JsonKeyValueView(title = "Server config", core = core, path = "admin/config", onBack = back)
        }
        SettingsRoute.ConfigOverrides -> ConfigOverridesView(core, back)
        SettingsRoute.Opml -> OpmlView(core, back)
        SettingsRoute.DbTransfer -> DbTransferView(core, back)
        SettingsRoute.LocalData -> LocalDataView(core, back)
    }
}

@Composable
private fun SettingsRoot(core: HalogenCore, nav: NavModel, push: (SettingsRoute) -> Unit) {
    var confirmSignOut by remember { mutableStateOf(false) }
    val serverLabel = when (val kind = core.account?.kind) {
        is AccountContext.Kind.Embedded -> "This device (embedded)"
        is AccountContext.Kind.Remote -> kind.serverUrl
        null -> "—"
    }
    val statusLabel = when (core.connection.status) {
        ConnectionMonitor.Status.Online -> "Online"
        ConnectionMonitor.Status.Offline -> "Offline"
        ConnectionMonitor.Status.Unknown -> "Checking…"
    }

    // Root screen: the shared navbar (online dot + account menu), iOS parity.
    Scaffold(topBar = { HalogenNavbar(core = core, title = "Settings") }) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            // A discarded change must stay discoverable after the toast is gone.
            val failures = core.syncFailures
            if (failures != null && failures.count > 0) {
                SettingsGroup {
                    SettingsRow(
                        onClick = { push(SettingsRoute.SyncFailures) },
                        modifier = Modifier.testTag("settings-sync-failures"),
                    ) {
                        Icon(
                            halogenIcon("exclamationmark.arrow.triangle.2.circlepath"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                        Text(
                            "${failures.count} change${if (failures.count == 1) "" else "s"} couldn't sync",
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.weight(1f),
                        )
                        RowChevron()
                    }
                }
            }

            SettingsGroup(header = "Account") {
                if (core.isEmbeddedAccount) {
                    // Embedded credentials are app-managed (silent login) —
                    // renaming would desync the stored secrets.
                    LabeledRow("User", core.account?.username ?: "—")
                } else {
                    SettingsRow(onClick = { push(SettingsRoute.UsernameEdit) }) {
                        Text("User", Modifier.weight(1f))
                        RowValue(core.account?.username ?: "—")
                        RowChevron()
                    }
                }
                LabeledRow("Server", serverLabel)
                SettingsRow {
                    Text("Status", Modifier.weight(1f))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        OnlineDot(status = core.connection.status)
                        RowValue(statusLabel)
                    }
                }
                NavRow("Accounts") { push(SettingsRoute.Accounts) }
            }

            val prefs = core.models?.prefs
            if (prefs != null) {
                SettingsGroup(header = "UI") {
                    PickerRow(
                        "Size",
                        selected = prefs.prefs.uiSize,
                        options = ClientPrefs.UISize.entries,
                        optionLabel = { it.label },
                    ) { new -> prefs.update { it.copy(uiSize = new) } }
                    PickerRow(
                        "Theme",
                        selected = prefs.prefs.theme,
                        options = ClientPrefs.AppTheme.entries,
                        optionLabel = { it.label },
                    ) { new -> prefs.update { it.copy(theme = new) } }
                }

                SettingsGroup(
                    header = "Playback",
                    footer = if (core.isEmbeddedAccount)
                        "The embedded server always streams — its media already lives on this device."
                    else null,
                ) {
                    if (core.isEmbeddedAccount) {
                        LabeledRow("Playback source", "Stream only")
                    } else {
                        PickerRow(
                            "Playback source",
                            selected = prefs.prefs.playbackStrategy,
                            options = ClientPrefs.PlaybackStrategy.entries,
                            optionLabel = { it.label },
                        ) { new -> prefs.update { it.copy(playbackStrategy = new) } }
                    }
                    PickerRow(
                        "Skip forward",
                        selected = prefs.prefs.skipForwardSecs,
                        options = listOf(15, 30, 45, 60),
                        optionLabel = { "${it}s" },
                    ) { new -> prefs.update { it.copy(skipForwardSecs = new) } }
                    PickerRow(
                        "Skip back",
                        selected = prefs.prefs.skipBackSecs,
                        options = listOf(10, 15, 30, 45),
                        optionLabel = { "${it}s" },
                    ) { new -> prefs.update { it.copy(skipBackSecs = new) } }
                    PickerRow(
                        "Default speed",
                        selected = prefs.prefs.defaultRate,
                        options = ClientPrefs.playbackRates,
                        optionLabel = { rateLabel(it) },
                    ) { new -> prefs.update { it.copy(defaultRate = new) } }
                    ToggleRow(
                        "Auto-play next in queue",
                        checked = prefs.prefs.autoAdvance,
                    ) { new -> prefs.update { it.copy(autoAdvance = new) } }
                    ToggleRow(
                        "Next/previous track buttons skip within the episode (for Bluetooth devices without seek buttons)",
                        checked = prefs.prefs.mediaNextPrevSeek,
                    ) { new -> prefs.update { it.copy(mediaNextPrevSeek = new) } }
                    ToggleRow(
                        "Add to front of queue",
                        checked = prefs.prefs.addToQueueFront,
                    ) { new -> prefs.update { it.copy(addToQueueFront = new) } }
                    PickerRow(
                        "Sleep timer",
                        selected = prefs.prefs.defaultSleepMinutes,
                        options = ClientPrefs.sleepDurations,
                        optionLabel = { "$it min" },
                    ) { new -> prefs.update { it.copy(defaultSleepMinutes = new) } }
                    ToggleRow(
                        "Sleep timer by default",
                        checked = prefs.prefs.sleepByDefault,
                    ) { new -> prefs.update { it.copy(sleepByDefault = new) } }
                }

                // Embedded accounts have no device downloads to tune — the
                // media already lives in the on-device server.
                if (!core.isEmbeddedAccount) {
                    SettingsGroup(
                        header = "Downloads",
                        footer = "Device downloads fetch in chunks — each finished chunk is saved progress, so slow or flaky connections resume instead of restarting. Parallel chunks fetch concurrently within one download.",
                    ) {
                        PickerRow(
                            "Chunk size",
                            selected = prefs.prefs.downloadChunkKiB,
                            options = ClientPrefs.downloadChunkKiBOptions,
                            optionLabel = { kib ->
                                if (kib == 0) "No chunking (whole file)" else "${kib / 1024} MB"
                            },
                        ) { new -> prefs.update { it.copy(downloadChunkKiB = new) } }
                        // No chunking = a single request — nothing to parallelize.
                        PickerRow(
                            "Parallel chunks",
                            selected = prefs.prefs.downloadParallelism,
                            options = ClientPrefs.downloadParallelisms,
                            enabled = prefs.prefs.downloadChunkKiB != 0,
                            optionLabel = { "$it" },
                        ) { new -> prefs.update { it.copy(downloadParallelism = new) } }
                    }
                }
            }

            SettingsGroup(header = "Navigation") {
                NavRow("Configure dock") { push(SettingsRoute.ConfigureDock) }
                if (core.models?.swipes != null) {
                    NavRow("Configure swipes") { push(SettingsRoute.ConfigureSwipes) }
                }
            }

            // Embedded accounts sign in silently with app-managed secrets —
            // a user-set password would desync them.
            if (!core.isEmbeddedAccount) {
                SettingsGroup(header = "Security") {
                    NavRow("Change password") { push(SettingsRoute.ChangePassword) }
                }
            }

            // Admin-only server surfaces — non-admins would only hit 403s here.
            if (core.isAdmin) {
                SettingsGroup(header = "Server") {
                    NavRow("Server errors") { push(SettingsRoute.ServerErrors) }
                    NavRow("View config") { push(SettingsRoute.ViewConfig) }
                    NavRow("Config overrides") { push(SettingsRoute.ConfigOverrides) }
                    NavRow("OPML import / export") { push(SettingsRoute.Opml) }
                    NavRow("Database export / import") { push(SettingsRoute.DbTransfer) }
                }
            }

            SettingsGroup(header = "Local data") {
                NavRow("Storage & purge") { push(SettingsRoute.LocalData) }
            }

            SettingsGroup(
                footer = "Cached data stays on this device and reappears when the same account signs in again.",
            ) {
                SettingsRow(onClick = { confirmSignOut = true }) {
                    Text("Sign out", color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }

    if (confirmSignOut) {
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            title = { Text("Sign out?") },
            text = { Text("You'll return to the landing page.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmSignOut = false
                    core.signOut()
                }) { Text("Sign out", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") }
            },
        )
    }
}

/// Configure the dock: reorder destinations and toggle visibility — first
/// `NavModel.DOCK_SLOTS` visible items form the dock, More is always present,
/// Settings can't be hidden.
@Composable
fun ConfigureDockView(nav: NavModel, isAdmin: Boolean, onBack: () -> Unit) {
    // Non-admins never see the admin-only rows; their stored positions are
    // carried through unchanged on commit (the web's merge_nav_keys rule).
    val configurable = nav.config.order.filter { isAdmin || !it.adminOnly }
    val working = remember { mutableStateListOf<BuiltinNav>() }
    var dragged by remember { mutableStateOf<BuiltinNav?>(null) }
    var dragOffset by remember { mutableFloatStateOf(0f) }
    val rowHeights = remember { mutableStateMapOf<BuiltinNav, Int>() }
    val rows = if (dragged != null) working.toList() else configurable

    fun shuffle(item: BuiltinNav) {
        while (true) {
            val index = working.indexOf(item)
            if (index < 0) return
            val next = working.getOrNull(index + 1)
            val prev = working.getOrNull(index - 1)
            val nextH = next?.let { rowHeights[it] } ?: 0
            val prevH = prev?.let { rowHeights[it] } ?: 0
            when {
                next != null && nextH > 0 && dragOffset > nextH / 2f -> {
                    working[index] = next
                    working[index + 1] = item
                    dragOffset -= nextH
                }
                prev != null && prevH > 0 && dragOffset < -prevH / 2f -> {
                    working[index] = prev
                    working[index - 1] = item
                    dragOffset += prevH
                }
                else -> return
            }
        }
    }

    fun commitDrag() {
        val config = nav.config
        nav.update(config.copy(order = working.toList() + config.order.filter { it !in working }))
        dragged = null
        dragOffset = 0f
    }

    SettingsScaffold(title = "Configure dock", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(
                footer = "Drag to reorder. The dock shows the first ${NavModel.DOCK_SLOTS} visible items; everything visible appears in More. Settings can't be hidden.",
            ) {
                for (item in rows) {
                    androidx.compose.runtime.key(item) {
                        SettingsRow(
                            modifier = Modifier
                                .zIndex(if (dragged == item) 1f else 0f)
                                .graphicsLayer {
                                    translationY = if (dragged == item) dragOffset else 0f
                                }
                                .onSizeChanged { rowHeights[item] = it.height },
                        ) {
                            Icon(
                                halogenIcon(item.systemImage),
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.size(24.dp),
                            )
                            Text(item.label, Modifier.weight(1f))
                            Switch(
                                checked = item !in nav.config.hidden,
                                enabled = item != BuiltinNav.Settings,
                                onCheckedChange = { visible ->
                                    val config = nav.config
                                    val updated = if (visible) {
                                        config.copy(hidden = config.hidden.filterNot { it == item })
                                    } else if (item != BuiltinNav.Settings) {
                                        config.copy(hidden = config.hidden + item)
                                    } else config
                                    nav.update(updated)
                                },
                                modifier = Modifier.testTag("nav-toggle-${item.token}"),
                            )
                            Icon(
                                Icons.Rounded.DragHandle,
                                contentDescription = "Reorder",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.pointerInput(item) {
                                    detectDragGestures(
                                        onDragStart = {
                                            working.clear()
                                            working.addAll(
                                                nav.config.order.filter { isAdmin || !it.adminOnly })
                                            dragged = item
                                            dragOffset = 0f
                                        },
                                        onDrag = { change, amount ->
                                            change.consume()
                                            dragOffset += amount.y
                                            shuffle(item)
                                        },
                                        onDragEnd = { commitDrag() },
                                        onDragCancel = { commitDrag() },
                                    )
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}

// ── shared settings scaffolding (grouped-list idiom for this package) ────────

/// Inline-title scaffold with hardware-back wiring for pushed screens.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SettingsScaffold(
    title: String,
    onBack: (() -> Unit)?,
    actions: @Composable RowScope.() -> Unit = {},
    content: @Composable (PaddingValues) -> Unit,
) {
    if (onBack != null) BackHandler(onBack = onBack)
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
                actions = actions,
            )
        },
    ) { padding -> content(padding) }
}

/// One grouped section: optional header/footer around a rounded card of rows.
@Composable
internal fun SettingsGroup(
    header: String? = null,
    footer: String? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        if (header != null) {
            Text(
                header,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 16.dp, bottom = 6.dp),
            )
        }
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerLow,
        ) {
            Column(content = content)
        }
        if (footer != null) {
            Text(
                footer,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 16.dp, top = 6.dp),
            )
        }
    }
}

@Composable
internal fun SettingsRow(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) {
    Row(
        modifier
            .fillMaxWidth()
            .then(
                if (onClick != null) Modifier.clickable(enabled = enabled, onClick = onClick)
                else Modifier)
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .heightIn(min = 44.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        content()
    }
}

@Composable
internal fun RowScope.RowValue(
    value: String,
    // Middle ellipsis suits URLs (host + port both survive); picker labels
    // want the END cut ("Medium (125%, …" beats "Medium (…, default)").
    overflow: TextOverflow = TextOverflow.MiddleEllipsis,
) {
    Text(
        value,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        overflow = overflow,
        textAlign = TextAlign.End,
        // fill = false: takes up to what the (unweighted) label leaves, but
        // shrinks to its text — filling starved labels inside nested rows.
        modifier = Modifier.weight(1f, fill = false),
    )
}

@Composable
internal fun RowChevron() {
    Icon(
        halogenIcon("chevron.right"),
        contentDescription = null,
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.size(18.dp),
    )
}

/// iOS `LabeledContent` — static label + secondary value.
@Composable
internal fun LabeledRow(label: String, value: String) {
    SettingsRow {
        Text(label, maxLines = 1)
        RowValue(value)
    }
}

/// iOS `NavigationLink` row — title + chevron.
@Composable
internal fun NavRow(title: String, onClick: () -> Unit) {
    SettingsRow(onClick = onClick) {
        Text(title, Modifier.weight(1f))
        RowChevron()
    }
}

@Composable
internal fun ToggleRow(
    title: String,
    checked: Boolean,
    enabled: Boolean = true,
    onCheckedChange: (Boolean) -> Unit,
) {
    SettingsRow {
        Text(title, Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onCheckedChange, enabled = enabled)
    }
}

/// iOS inline `Picker` — current value on the row, options in a menu.
@Composable
internal fun <T> PickerRow(
    label: String,
    selected: T,
    options: List<T>,
    enabled: Boolean = true,
    optionLabel: (T) -> String,
    onSelect: (T) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        SettingsRow(onClick = { expanded = true }, enabled = enabled) {
            Text(
                label,
                maxLines = 1,
                color = if (enabled) Color.Unspecified
                else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // Value and its menu chevron travel together, tight: a full
            // 12dp gap here cost the value its last characters.
            Row(
                Modifier.weight(1f),
                horizontalArrangement = Arrangement.spacedBy(2.dp, Alignment.End),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RowValue(optionLabel(selected), overflow = TextOverflow.Ellipsis)
                Icon(
                    halogenIcon("chevron.up.chevron.down"),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(12.dp),
                )
            }
        }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            for (option in options) {
                DropdownMenuItem(
                    text = { Text(optionLabel(option)) },
                    trailingIcon = {
                        if (option == selected) {
                            Icon(halogenIcon("checkmark"), contentDescription = null)
                        }
                    },
                    onClick = {
                        expanded = false
                        onSelect(option)
                    },
                )
            }
        }
    }
}

@Composable
internal fun FootnoteText(
    text: String,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    modifier: Modifier = Modifier,
) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color, modifier = modifier)
}

/// Full-width submit button with the iOS in-place ProgressView while busy.
@Composable
internal fun BusyButton(
    title: String,
    busy: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Button(
        onClick = onClick,
        enabled = enabled && !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        if (busy) {
            CircularProgressIndicator(
                modifier = Modifier.size(16.dp),
                color = MaterialTheme.colorScheme.onPrimary,
                strokeWidth = 2.dp,
            )
        } else {
            Text(title)
        }
    }
}

/// Form text field (iOS TextField/SecureField): single line, no autocorrect.
@Composable
internal fun SettingsTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    keyboardType: KeyboardType = KeyboardType.Text,
    secure: Boolean = false,
    trailingIcon: @Composable (() -> Unit)? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label) },
        singleLine = true,
        visualTransformation =
            if (secure) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            capitalization = capitalization,
            keyboardType = if (secure) KeyboardType.Password else keyboardType,
            autoCorrectEnabled = false,
        ),
        trailingIcon = trailingIcon,
        modifier = modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
    )
}

/// iOS `String(format: "%g×", rate)` — shortest decimal form.
internal fun rateLabel(rate: Float): String {
    val text = if (rate % 1f == 0f) rate.toInt().toString() else rate.toString()
    return "$text×"
}
