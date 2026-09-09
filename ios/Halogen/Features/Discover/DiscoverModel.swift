import Foundation
import Observation

/// Online podcast search + subscribe. No caching — discover is online-only on
/// every platform. The user's disabled-provider set persists in ClientPrefs
/// (web disabled_discover_providers) and narrows the search fan-out.
@MainActor
@Observable
final class DiscoverModel {
    private unowned let core: HalogenCore

    var query = "" {
        didSet {
            if query != oldValue {
                searchGeneration += 1
                pages.invalidate()
            }
        }
    }
    var mode: DiscoverSearchMode = .podcast {
        didSet {
            if mode != oldValue {
                searchGeneration += 1
                pages.invalidate(clear: true)
            }
        }
    }
    private var searchGeneration = 0
    let pages: DiscoverResultsModel
    var results: [DiscoverResultItem] { pages.podcasts }
    var searching: Bool { pages.isLoading }
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
        pages = DiscoverResultsModel(
            fetch: { [unowned core] key, cursor in try await core.discoverPage(key, cursor: cursor) },
            errorMessage: FriendlyError.message)
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
        searchGeneration += 1
        pages.invalidate(clear: true)
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
        searchGeneration += 1
        let request = searchGeneration
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
            guard request == searchGeneration else { return }
            if providers.isEmpty {
                if providerError {
                    error = "Search providers failed to load — retry above."
                }
                return
            }
        }
        guard request == searchGeneration else { return }
        let enabled = enabledProviders
        guard !enabled.isEmpty else {
            error = "Enable at least one provider"
            return
        }
        let selected = mode == .episode ? enabled.filter { $0 == .itunes } : enabled
        guard !selected.isEmpty else {
            error = "Enable iTunes to search episodes. gpodder supports podcast search only."
            return
        }
        error = nil
        await pages.search(DiscoverSearchKey(query: q, mode: mode, providers: selected))
        refreshSubscribed()
    }

    func subscribe(_ item: DiscoverResultItem) async {
        // Durable subscribe (web: OutboxOp::Subscribe) — queues offline and
        // survives restarts; the library refresh reconciles once it drains.
        guard
            await core.ensureQueued(
                .subscribe(
                    feedUrl: item.feed_url,
                    title: item.title,
                    description: (item.description?.isEmpty ?? true) ? nil : item.description
                ))
        else { return }
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
