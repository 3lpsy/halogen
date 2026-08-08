import SwiftUI

/// Podcast management destinations — menu items can't push NavigationLinks,
/// so the podcast page and the library row menu both drive this via
/// `navigationDestination(item:)`.
enum PodcastManageRoute: String, Identifiable, Hashable {
    case edit
    case downloadConfig
    case autoPlaylists
    case metadata

    var id: String { rawValue }
}

/// The shared menu items (podcast page toolbar + library row ellipsis).
struct PodcastManageMenu: View {
    @Binding var manage: PodcastManageRoute?
    @Binding var confirmDelete: Bool

    var body: some View {
        Button {
            manage = .edit
        } label: {
            Label("Edit podcast", systemImage: "pencil")
        }
        Button {
            manage = .downloadConfig
        } label: {
            Label("Download config", systemImage: "arrow.down.circle.dotted")
        }
        Button {
            manage = .autoPlaylists
        } label: {
            Label("Auto-playlists", systemImage: "music.note.list")
        }
        Button {
            manage = .metadata
        } label: {
            Label("Metadata", systemImage: "info.circle")
        }
        Divider()
        Button(role: .destructive) {
            confirmDelete = true
        } label: {
            Label("Delete podcast", systemImage: "trash")
        }
    }
}

/// Resolves a manage route to its screen.
struct PodcastManageScreen: View {
    let route: PodcastManageRoute
    let core: HalogenCore
    let podcast: PodcastData

    var body: some View {
        switch route {
        case .edit:
            PodcastEditView(core: core, podcast: podcast)
        case .downloadConfig:
            PodcastConfigFormView(core: core, podcast: podcast)
        case .autoPlaylists:
            AutoPlaylistsView(core: core, podcast: podcast)
        case .metadata:
            JSONKeyValueView(
                title: "Metadata", core: core, path: "podcasts/\(podcast.id)",
                cacheKey: CacheKey.metadata("podcasts/\(podcast.id)"))
        }
    }
}
