package org.fgsec.halogen

import android.app.Activity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import org.fgsec.halogen.components.LocalNavigator
import org.fgsec.halogen.components.Navigator
import org.fgsec.halogen.components.ToastHost
import org.fgsec.halogen.components.appDestinations
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.components.HalogenTheme
import org.fgsec.halogen.core.BuiltinNav
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.Models
import org.fgsec.halogen.features.auth.ConnectView
import org.fgsec.halogen.features.more.BuiltinDestination
import org.fgsec.halogen.features.more.MoreView
import org.fgsec.halogen.features.player.MiniPlayerBar
import org.fgsec.halogen.features.settings.LocalDataView

/// Root: resumes the stored session (or lands on auth), then hands over to
/// the tab dock.
@Composable
fun RootView(core: HalogenCore) {
    var showRecoveryPurge by remember { mutableStateOf(false) }
    val prefs = core.models?.prefs?.prefs
    val dark = prefs?.theme?.isDark ?: true
    val fontScale = prefs?.uiSize?.fontScale ?: 1.0f

    HalogenTheme(darkTheme = dark) {
        // System-bar ICON appearance follows the in-app theme: the window
        // theme is dark (light icons), so the Light theme would otherwise
        // show white-on-white status icons.
        val view = LocalView.current
        if (!view.isInEditMode) {
            SideEffect {
                (view.context as? Activity)?.window?.let { window ->
                    val bars = WindowCompat.getInsetsController(window, view)
                    bars.isAppearanceLightStatusBars = !dark
                    bars.isAppearanceLightNavigationBars = !dark
                }
            }
        }
        val base = LocalDensity.current
        CompositionLocalProvider(
            LocalDensity provides Density(base.density, base.fontScale * fontScale)
        ) {
            Surface(modifier = Modifier.fillMaxSize()) {
                when (val phase = core.phase) {
                    is HalogenCore.Phase.Ready -> core.models?.let { models ->
                        // Keyed by account: a switch remounts the tabs (and
                        // their models) over the new namespace.
                        key(core.account?.namespace) {
                            Column {
                                core.storageFailure?.let { StorageFailureBanner(it) }
                                HomeTabs(core, models)
                            }
                        }
                    }
                    is HalogenCore.Phase.NeedsAuth -> ConnectView(core)
                    is HalogenCore.Phase.Failed -> BootFailure(
                        core = core, message = phase.message,
                        onPurge = { showRecoveryPurge = true },
                    )
                    is HalogenCore.Phase.Idle, is HalogenCore.Phase.Starting -> BootSplash()
                }
                ToastHost()
            }
            if (showRecoveryPurge) {
                // The web's standalone /cache-control page renders even
                // mid-failure: a wedged store or a corrupt embedded library
                // must always be purgeable in-app.
                LocalDataView(core = core, onBack = { showRecoveryPurge = false })
            }
        }
    }

    LaunchedEffect(Unit) { core.boot() }
    val lifecycle = ProcessLifecycleOwner.get().lifecycle
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_START) {
                core.scope.launch { core.foregroundSync() }
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
}

/// One tab's root marker route (typed nav needs a serializable start).
@Serializable
private data object TabRoot

/// A pushed More-menu destination (BuiltinNav rides by token).
@Serializable
private data class MoreItem(val token: String)

/// The tab dock: the first N visible nav destinations plus the always-present
/// More tab, mirroring the web's dock + `/menu` model. 4 slots + More, as iOS.
@Composable
private fun HomeTabs(core: HalogenCore, models: Models) {
    var selection by rememberSaveable {
        mutableStateOf(DebugHooks.tab ?: BuiltinNav.Queue.token)
    }
    // Re-tapping the SELECTED tab pops its stack to root (iOS TabView
    // behavior). Ticks, not booleans: each re-tap must fire even mid-settle.
    val resetTicks = remember { mutableStateMapOf<String, Int>() }
    val selectTab = { token: String ->
        if (selection == token) resetTicks[token] = (resetTicks[token] ?: 0) + 1
        else selection = token
    }
    val dockItems = models.nav.dockItems(isAdmin = core.isAdmin)

    Scaffold(
        bottomBar = {
            // Chrome, not content: the dock opts out of the UI-size scale,
            // which ellipsized its labels to "Podcas…" at 125%.
            CompositionLocalProvider(
                LocalDensity provides Density(LocalDensity.current.density, 1f)
            ) {
            NavigationBar {
                for (item in dockItems) {
                    NavigationBarItem(
                        selected = selection == item.token,
                        onClick = { selectTab(item.token) },
                        icon = { Icon(halogenIcon(item.systemImage), contentDescription = item.label) },
                        // Smallest label style, ellipsized not wrapped.
                        label = {
                            Text(
                                item.label, maxLines = 1, overflow = TextOverflow.Ellipsis,
                                style = MaterialTheme.typography.labelSmall,
                            )
                        },
                        modifier = Modifier.testTag("tab-${item.token}"),
                    )
                }
                NavigationBarItem(
                    selected = selection == "more",
                    onClick = { selectTab("more") },
                    icon = { Icon(halogenIcon("ellipsis"), contentDescription = "More") },
                    label = {
                        Text(
                            "More", maxLines = 1, overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.labelSmall,
                        )
                    },
                    modifier = Modifier.testTag("tab-more"),
                )
            }
            }
        },
    ) { padding ->
        // Every dock tab keeps its own stack; switching tabs preserves it.
        // Only the selected tab is composed, but its saveable state (NavHost
        // back stack, scroll positions, edit modes) is parked in the holder
        // and restored on return — iOS keeps every TabStack mounted.
        val stateHolder = rememberSaveableStateHolder()
        Column(Modifier.padding(padding)) {
            for (item in dockItems) {
                if (selection == item.token) {
                    stateHolder.SaveableStateProvider(item.token) {
                        TabStack(core, models, resetTick = resetTicks[item.token] ?: 0) {
                            BuiltinDestination(item, core, models)
                        }
                    }
                }
            }
            if (selection == "more") {
                stateHolder.SaveableStateProvider("more") {
                    MoreTabStack(core, models, resetTick = resetTicks["more"] ?: 0)
                }
            }
        }
    }

    GlobalPlaylistPickDialog(models)
    LaunchedEffect(Unit) { debugAutoplayHooks(core, models) }
}

/// The More tab's stack: same shell, plus the menu's push-by-token entry.
@Composable
private fun MoreTabStack(core: HalogenCore, models: Models, resetTick: Int = 0) {
    TabStack(core, models, moreMenu = true, resetTick = resetTick) {}
}

/// One tab's NavHost: owns the tab's Navigator and injects it so rows/menus
/// can push AppRoutes without NavigationLinks. The mini player rides the
/// stack, not its root — pushed screens keep it.
@Composable
private fun TabStack(
    core: HalogenCore, models: Models, moreMenu: Boolean = false, resetTick: Int = 0,
    content: @Composable () -> Unit,
) {
    val controller = rememberNavController()
    val navigator = remember { Navigator(controller) }
    // Selected-tab re-tap: unwind this tab's stack to its root.
    LaunchedEffect(resetTick) {
        if (resetTick > 0) controller.popBackStack(TabRoot, inclusive = false)
    }
    CompositionLocalProvider(LocalNavigator provides navigator) {
        Column(Modifier.fillMaxSize()) {
            Column(Modifier.weight(1f)) {
                NavHost(navController = controller, startDestination = TabRoot) {
                    composable<TabRoot> {
                        if (moreMenu) MoreView(core, models, onOpen = { controller.navigate(MoreItem(it.token)) })
                        else content()
                    }
                    composable<MoreItem> { entry ->
                        val token = entry.toRoute<MoreItem>().token
                        val item = BuiltinNav.entries.firstOrNull { it.token == token }
                        if (item != null) BuiltinDestination(item, core, models)
                    }
                    appDestinations(core, models)
                }
            }
            MiniPlayerBar(player = models.player, core = core)
        }
    }
}

/// Global add-to-playlist picker: swipe/bulk "Add to playlist" queues
/// episodes on PlaylistsModel.pendingPick and this dialog resolves the
/// choice wherever the user is.
@Composable
private fun GlobalPlaylistPickDialog(models: Models) {
    val pending = models.playlists.pendingPick
    if (pending.isEmpty()) return
    val title = if (pending.size == 1) "Add “${pending.first().title}” to playlist"
    else "Add ${pending.size} episodes to playlist"
    AlertDialog(
        onDismissRequest = { models.playlists.pendingPick = emptyList() },
        title = { Text(title) },
        text = {
            Column {
                for (playlist in models.playlists.playlists.filter { !it.is_default }) {
                    TextButton(
                        onClick = {
                            for (episode in pending) models.playlists.add(episode, playlist)
                            models.playlists.pendingPick = emptyList()
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(playlist.name) }
                }
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = { models.playlists.pendingPick = emptyList() }) { Text("Cancel") }
        },
    )
}

/// DEBUG smoke-test hooks: HALOGEN_AUTOPLAY / HALOGEN_AUTODOWNLOAD exercise
/// streaming and the chunk/resume loop headlessly.
private suspend fun debugAutoplayHooks(core: HalogenCore, models: Models) {
    DebugHooks.autoplay?.let { id ->
        runCatching { core.episodeDetail(id) }.getOrNull()?.let { models.player.play(it) }
    }
    DebugHooks.autodownload?.let { id ->
        runCatching { core.episodeDetail(id) }.getOrNull()?.let { models.device.download(it) }
    }
}

/// Never a dead end: retry the boot, purge local data, or sign out.
@Composable
private fun BootFailure(core: HalogenCore, message: String, onPurge: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon("xmark.octagon"), contentDescription = null,
            modifier = Modifier.size(48.dp), tint = MaterialTheme.colorScheme.error,
        )
        Text("Couldn't start", style = MaterialTheme.typography.titleLarge)
        Text(
            message, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = { core.scope.launch { core.retryBoot() } }) { Text("Try again") }
        OutlinedButton(onClick = onPurge) { Text("Local data…") }
        TextButton(onClick = { core.signOut() }) {
            Text("Sign out", color = MaterialTheme.colorScheme.error)
        }
    }
}

/// Persistent warning when the account's LocalStore failed to open.
@Composable
private fun StorageFailureBanner(message: String) {
    Surface(color = MaterialTheme.colorScheme.error.copy(alpha = 0.88f)) {
        Text(
            message,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onError,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
        )
    }
}

/// Shown while a stored session resumes.
@Composable
private fun BootSplash() {
    Column(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon("waveform.circle.fill"), contentDescription = null,
            modifier = Modifier.size(56.dp), tint = MaterialTheme.colorScheme.primary,
        )
        Text("Halogen", style = MaterialTheme.typography.headlineLarge)
        CircularProgressIndicator()
    }
}
