package org.fgsec.halogen.features.discover

import org.fgsec.halogen.core.ensureQueued
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.DiscoverProviderInfo
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.DiscoverProvider
import org.fgsec.halogen.wire.DiscoverResultItem

/// Online podcast search + subscribe — online-only on every platform (results come
/// from third-party directories the server proxies live). The user's disabled-provider
/// set persists in ClientPrefs and narrows the search fan-out.
class DiscoverModel(private val core: HalogenCore) {

    var query: String by mutableStateOf("")
    var results: List<DiscoverResultItem> by mutableStateOf(emptyList())
        private set
    var searching: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set
    var subscribedFeeds: Set<String> by mutableStateOf(emptySet())
        private set
    /// The server's provider list (chips). Empty = not loaded yet.
    var providers: List<DiscoverProviderInfo> by mutableStateOf(emptyList())
        private set
    /// A failed provider fetch must not silently disable Search forever —
    /// it surfaces a Retry instead (web: provider_error).
    var providerError: Boolean by mutableStateOf(false)
        private set
    private var fetchingProviders = false
    /// Stale-list guard: a slow older search must not clobber newer results.
    private var generation = 0

    val isOffline: Boolean get() = core.isOffline

    /// Fetch the provider list once (for the toggle chips); retried via the
    /// error banner's Retry and on later appearances while it stays empty.
    suspend fun loadProviders() {
        if (providers.isNotEmpty() || fetchingProviders || core.isOffline) return
        fetchingProviders = true
        try {
            providers = core.discoverProviders().providers
            providerError = false
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("discover: provider fetch failed — ${e::class.simpleName}: ${e.message}")
            providerError = true
        } finally {
            fetchingProviders = false
        }
    }

    fun isEnabled(provider: DiscoverProvider): Boolean =
        provider.string !in
            (core.models?.prefs?.prefs?.disabledDiscoverProviders ?: emptyList())

    /// Toggle a provider chip; the disabled set persists across restarts.
    fun toggleProvider(provider: DiscoverProvider) {
        core.models?.prefs?.update { prefs ->
            val disabled = prefs.disabledDiscoverProviders
            prefs.copy(
                disabledDiscoverProviders =
                    if (provider.string in disabled) disabled - provider.string
                    else disabled + provider.string
            )
        }
    }

    /// The providers a search queries: available ones the user hasn't
    /// toggled off (web: enabled_providers).
    private val enabledProviders: List<DiscoverProvider>
        get() = providers.filter { it.available && isEnabled(it.id) }.map { it.id }

    suspend fun search() {
        val q = query.trim()
        // The web requires >= 2 chars before searching.
        if (q.length < 2) return
        if (core.isOffline) {
            error = "Discover needs an internet connection."
            return
        }
        // Don't search until the provider list has loaded: empty means
        // "still loading" (wait) or a FAILED load, which deserves feedback
        // instead of a dead search (web rule).
        if (providers.isEmpty()) {
            loadProviders()
            if (providers.isEmpty()) {
                if (providerError) {
                    error = "Search providers failed to load — retry above."
                }
                return
            }
        }
        val enabled = enabledProviders
        if (enabled.isEmpty()) {
            error = "Enable at least one provider"
            return
        }
        generation += 1
        val mine = generation
        searching = true
        try {
            val data = core.discoverSearch(query = q, providers = enabled)
            if (mine != generation) return
            results = data.items
            // Per-provider partial failures surface even when other
            // providers returned results (web: one toast per failed provider).
            val errors = data.errors
            if (!errors.isNullOrEmpty()) {
                error = errors.joinToString("\n") {
                    "${providerLabel(it.provider)} search failed: ${it.message}"
                }
            }
            refreshSubscribed()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (mine == generation) error = FriendlyError.message(e)
        } finally {
            if (mine == generation) searching = false
        }
    }

    suspend fun subscribe(item: DiscoverResultItem) {
        // Durable subscribe (web: OutboxOp::Subscribe) — queues offline and
        // survives restarts; the library refresh reconciles once it drains.
        if (!core.ensureQueued(
            OutboxOp.Kind.Subscribe(
                feedUrl = item.feed_url,
                title = item.title,
                description = item.description?.takeIf { it.isNotEmpty() },
            ))) return
        subscribedFeeds = subscribedFeeds + item.feed_url
        core.models?.podcasts?.refresh()
    }

    fun isSubscribed(item: DiscoverResultItem): Boolean = item.feed_url in subscribedFeeds

    /// A queued subscribe was dead-lettered: un-flip the row's optimistic
    /// "Subscribed" checkmark (which otherwise survived until relaunch).
    fun noteSubscribeFailed(feedUrl: String) {
        subscribedFeeds = subscribedFeeds - feedUrl
    }

    fun clearError() {
        error = null
    }

    fun retryProviders() {
        providerError = false
        core.scope.launch { loadProviders() }
    }

    /// Seed the subscribed set from the cached library so already-subscribed
    /// feeds show the checkmark.
    private fun refreshSubscribed() {
        val known = core.models?.podcasts?.podcasts?.map { it.feed_url } ?: emptyList()
        subscribedFeeds = subscribedFeeds + known
    }

    fun providerLabel(p: DiscoverProvider): String =
        providers.firstOrNull { it.id == p }?.label
            ?: if (p == DiscoverProvider.Itunes) "iTunes" else "gpodder.net"
}
