package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import org.fgsec.halogen.storage.LocalStore

/// Built-in navigation destinations — mirrors the web's `BuiltinNav`
/// (webui/config nav.rs), same snake_case tokens so a future config sync
/// could share them.
@Serializable
enum class BuiltinNav(val token: String) {
    @SerialName("queue") Queue("queue"),
    @SerialName("latest") Latest("latest"),
    @SerialName("podcasts") Podcasts("podcasts"),
    @SerialName("playlists") Playlists("playlists"),
    @SerialName("downloads") Downloads("downloads"),
    @SerialName("discover") Discover("discover"),
    @SerialName("history") History("history"),
    @SerialName("settings") Settings("settings"),
    @SerialName("polling") Polling("polling"),
    @SerialName("device_logs") DeviceLogs("device_logs"),
    @SerialName("server_logs") ServerLogs("server_logs");

    val label: String
        get() = when (this) {
            Queue -> "Queue"
            Latest -> "Latest"
            Podcasts -> "Podcasts"
            Playlists -> "Playlists"
            Downloads -> "Downloads"
            Discover -> "Discover"
            History -> "History"
            Settings -> "Settings"
            Polling -> "Polling"
            DeviceLogs -> "Device Logs"
            ServerLogs -> "Server Logs"
        }

    /// Admin-only destinations (web: `NavItem.admin_only` — Polling and
    /// Server Logs are dropped from every nav surface for non-admins).
    val adminOnly: Boolean
        get() = this == Polling || this == ServerLogs

    /// SF Symbol name — components/Icons.kt maps it to a Material icon.
    val systemImage: String
        get() = when (this) {
            Queue -> "list.bullet"
            Latest -> "clock"
            Podcasts -> "square.grid.2x2"
            Playlists -> "music.note.list"
            Downloads -> "arrow.down.circle"
            Discover -> "magnifyingglass"
            History -> "clock.arrow.circlepath"
            Settings -> "gear"
            Polling -> "arrow.triangle.2.circlepath"
            DeviceLogs -> "doc.text"
            ServerLogs -> "server.rack"
        }
}

/// Nav order/visibility — the web's `NavConfig` shape (order + hidden +
/// pinned playlist ids), persisted per account. The dock renders the first
/// `NavModel.DOCK_SLOTS` visible items plus the always-present More tab; the
/// More menu renders everything visible.
@Serializable
data class NavConfig(
    val order: List<BuiltinNav>,
    val hidden: List<BuiltinNav>,
    /// Playlist ids pinned as nav links (parity field; pin UI not built yet).
    val pinnedPlaylists: List<Int>,
) {
    /// Self-heal persisted orders that predate newly added builtins.
    /// Settings can never be hidden (same hard rule as the web).
    fun normalized(): NavConfig = copy(
        order = order + default.order.filter { it !in order },
        hidden = hidden.filter { it != BuiltinNav.Settings },
    )

    val visible: List<BuiltinNav>
        get() = order.filter { it !in hidden }

    companion object {
        val default = NavConfig(
            order = listOf(
                BuiltinNav.Queue, BuiltinNav.Latest, BuiltinNav.Podcasts,
                BuiltinNav.Playlists, BuiltinNav.Downloads, BuiltinNav.Discover,
                BuiltinNav.History, BuiltinNav.Settings, BuiltinNav.Polling,
                BuiltinNav.DeviceLogs, BuiltinNav.ServerLogs,
            ),
            hidden = listOf(BuiltinNav.Polling, BuiltinNav.DeviceLogs, BuiltinNav.ServerLogs),
            pinnedPlaylists = emptyList(),
        )
    }
}

/// Reactive holder + persistence for the account's NavConfig.
class NavModel(
    private val store: LocalStore?,
    private val scope: CoroutineScope = MainScope(),
) {
    var config: NavConfig by mutableStateOf(NavConfig.default)
        private set

    suspend fun load() {
        val saved = store?.load<NavConfig>(KEY) ?: return
        config = saved.normalized()
    }

    fun update(new: NavConfig) {
        val normalized = new.normalized()
        config = normalized
        scope.launch { store?.save(normalized, KEY) }
    }

    /// Every destination this user may see, in configured order — the web's
    /// `nav_items`: hidden dropped, admin-only dropped for non-admins.
    fun visibleItems(isAdmin: Boolean): List<BuiltinNav> =
        config.visible.filter { isAdmin || !it.adminOnly }

    /// The tab bar: first N visible destinations (More is appended by the UI).
    fun dockItems(isAdmin: Boolean): List<BuiltinNav> =
        visibleItems(isAdmin).take(DOCK_SLOTS)

    companion object {
        /// 4 destinations + More, as on iOS. Six slots fit physically but
        /// not legibly — the labels ellipsized.
        const val DOCK_SLOTS = 4
        private const val KEY = "nav-config"
    }
}
