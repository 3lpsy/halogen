import SwiftUI

/// The dock's always-present More tab — the full navigation menu (the web's
/// `/menu`): every visible destination in configured order. Destinations not
/// built natively yet are listed disabled so the gap is visible, not silent.
struct MoreView: View {
    let core: HalogenCore
    let models: Models

    var body: some View {
        List {
            Section {
                ForEach(models.nav.visibleItems(isAdmin: core.isAdmin)) { item in
                    if BuiltinDestination.isImplemented(item) {
                        NavigationLink(value: item) {
                            Label(item.label, systemImage: item.systemImage)
                        }
                    } else {
                        HStack {
                            Label(item.label, systemImage: item.systemImage)
                                .foregroundStyle(.tertiary)
                            Spacer()
                            Text("Soon")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
            } footer: {
                Text("Reorder or hide items in Settings → Configure dock.")
            }
        }
        .halogenNavbar(core: core)
        .navigationDestination(for: BuiltinNav.self) { item in
            BuiltinDestination(item: item, core: core, models: models)
        }
    }
}

/// Maps a nav destination to its screen — shared by the dock tabs and the
/// More menu so both always agree on what exists.
struct BuiltinDestination: View {
    let item: BuiltinNav
    let core: HalogenCore
    let models: Models

    static func isImplemented(_ item: BuiltinNav) -> Bool {
        switch item {
        case .queue, .latest, .podcasts, .playlists, .downloads, .settings,
            .discover, .history, .polling, .serverLogs, .deviceLogs:
            return true
        }
    }

    var body: some View {
        switch item {
        case .queue:
            QueueView(model: models.queue, core: core)
        case .latest:
            LatestView(model: models.latest, core: core)
        case .podcasts:
            PodcastsView(model: models.podcasts, core: core)
        case .playlists:
            PlaylistsView(model: models.playlists, core: core)
        case .downloads:
            DownloadsView(model: models.downloads, core: core)
        case .settings:
            SettingsView(core: core, nav: models.nav)
        case .discover:
            DiscoverView(model: models.discover, core: core)
        case .history:
            HistoryView(model: models.history, core: core)
        case .polling:
            PollingView(core: core)
        case .serverLogs:
            ServerLogsView(core: core)
        case .deviceLogs:
            DeviceLogsView(core: core)
        }
    }
}
