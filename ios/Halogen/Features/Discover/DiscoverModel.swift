import Foundation
import Observation

/// Online podcast search + subscribe. No caching — discover is online-only on
/// every platform. The user's disabled-provider set persists in ClientPrefs
/// (web disabled_discover_providers) and narrows the search fan-out.
@MainActor
@Observable
final class DiscoverModel {
    private unowned let core: HalogenCore

    var query = ""
    private(set) var results: [DiscoverResultItem] = []
    private(set) var searching = false
    private(set) var error: String?
    private(set) var subscribedFeeds: Set<String> = []
    /// The server's provider list (chips). Empty = not loaded yet.
    private(set) var providers: [DiscoverProviderInfo] = []
    /// A failed provider fetch must not silently disable Search forever —
    /// it surfaces a Retry instead (web: provider_error).
    private(set) var providerError = false
    private var fetchingProviders = false

    init(core: HalogenCore) {
        self.core = core
    }

    var isOffline: Bool { core.isOffline }

    /// Fetch the provider list once (for the toggle chips); retried via the
    /// error banner's Retry and on later appearances while it stays empty.
    func loadProviders() async {
        guard providers.isEmpty, !fetchingProviders, !core.isOffline else { return }
        fetchingProviders = true
        defer { fetchingProviders = false }
        do {
            providers = try await core.discoverProviders().providers
            providerError = false
        } catch {
            providerError = true
        }
    }

    func isEnabled(_ provider: DiscoverProvider) -> Bool {
        !(core.models?.prefs.prefs.disabledDiscoverProviders ?? [])
            .contains(provider.rawValue)
    }

    /// Toggle a provider chip; the disabled set persists across restarts.
    func toggleProvider(_ provider: DiscoverProvider) {
        core.models?.prefs.update { prefs in
            if let idx = prefs.disabledDiscoverProviders.firstIndex(of: provider.rawValue) {
                prefs.disabledDiscoverProviders.remove(at: idx)
            } else {
                prefs.disabledDiscoverProviders.append(provider.rawValue)
            }
        }
    }

    /// The providers a search queries: available ones the user hasn't
    /// toggled off (web: enabled_providers).
    private var enabledProviders: [DiscoverProvider] {
        providers.filter { $0.available && isEnabled($0.id) }.map(\.id)
    }

    func search() async {
        let q = query.trimmingCharacters(in: .whitespaces)
        // The web requires >= 2 chars before searching.
        guard q.count >= 2 else { return }
        guard !core.isOffline else {
            error = "Discover needs an internet connection."
            return
        }
        // Don't search until the provider list has loaded: empty means
        // "still loading" (wait) or a FAILED load, which deserves feedback
        // instead of a dead search (web rule).
        if providers.isEmpty {
            await loadProviders()
            if providers.isEmpty {
                if providerError {
                    error = "Search providers failed to load — retry above."
                }
                return
            }
        }
        let enabled = enabledProviders
        guard !enabled.isEmpty else {
            error = "Enable at least one provider"
            return
        }
        searching = true
        defer { searching = false }
        do {
            let data = try await core.discoverSearch(query: q, providers: enabled)
            results = data.items
            // Per-provider partial failures surface even when other
            // providers returned results (web: one toast per failed provider).
            if let errors = data.errors, !errors.isEmpty {
                error = errors
                    .map { "\(providerLabel($0.provider)) search failed: \($0.message)" }
                    .joined(separator: "\n")
            }
            refreshSubscribed()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }

    func subscribe(_ item: DiscoverResultItem) async {
        // Durable subscribe (web: OutboxOp::Subscribe) — queues offline and
        // survives restarts; the library refresh reconciles once it drains.
        await core.outbox?.enqueue(
            .subscribe(
                feedUrl: item.feed_url,
                title: item.title,
                description: (item.description?.isEmpty ?? true) ? nil : item.description
            ))
        subscribedFeeds.insert(item.feed_url)
        await core.models?.podcasts.refresh()
    }

    func isSubscribed(_ item: DiscoverResultItem) -> Bool {
        subscribedFeeds.contains(item.feed_url)
    }

    /// A queued subscribe was dead-lettered: un-flip the row's optimistic
    /// "Subscribed" checkmark (which otherwise survived until relaunch).
    func noteSubscribeFailed(feedUrl: String) {
        subscribedFeeds.remove(feedUrl)
    }

    func clearError() {
        error = nil
    }

    func retryProviders() {
        providerError = false
        Task { await loadProviders() }
    }

    /// Seed the subscribed set from the cached library so already-subscribed
    /// feeds show the checkmark.
    private func refreshSubscribed() {
        let known = core.models?.podcasts.podcasts.map(\.feed_url) ?? []
        subscribedFeeds.formUnion(known)
    }

    func providerLabel(_ p: DiscoverProvider) -> String {
        providers.first(where: { $0.id == p })?.label
            ?? (p == .itunes ? "iTunes" : "gpodder.net")
    }
}
