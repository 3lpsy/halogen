import Foundation
import Observation
import SwiftUI

/// Configurable episode swipe actions — the full web vocabulary (ui-listview
/// `SwipeAction::ALL`), one per edge per page. Toggles resolve against the
/// row's live state (web `perform_episode_action`); on embedded accounts
/// device-download actions remap to server ones (the server IS this device).
enum SwipeAction: String, Codable, CaseIterable, Identifiable {
    case none
    case play
    case stream
    case togglePlayed = "toggle_played"
    case markPlayed = "mark_played"
    case markUnplayed = "mark_unplayed"
    case resetProgress = "reset_progress"
    case addToQueue = "add_to_queue"
    case removeFromQueue = "remove_from_queue"
    case toggleQueue = "toggle_queue"
    /// Remove from *this list's* playlist (queue or the viewed playlist).
    case removeFromList = "remove_from_list"
    case addToPlaylist = "add_to_playlist"
    /// Keeps the pre-parity "download" token so saved prefs still decode.
    case downloadToDevice = "download"
    case redownloadDevice = "redownload_device"
    case removeDownload = "remove_download"
    case toggleDownload = "toggle_download"
    case downloadOnServer = "download_on_server"
    case removeFromServer = "remove_from_server"
    case toggleServerDownload = "toggle_server_download"

    var id: String { rawValue }

    var label: String {
        switch self {
        case .none: return "None"
        case .play: return "Play"
        case .stream: return "Stream"
        case .togglePlayed: return "Toggle played"
        case .markPlayed: return "Mark played"
        case .markUnplayed: return "Mark unplayed"
        case .resetProgress: return "Reset progress"
        case .addToQueue: return "Add to queue"
        case .removeFromQueue: return "Remove from queue"
        case .toggleQueue: return "Toggle queue"
        case .removeFromList: return "Remove from list"
        case .addToPlaylist: return "Add to playlist"
        case .downloadToDevice: return "Download"
        case .redownloadDevice: return "Re-download"
        case .removeDownload: return "Remove download"
        case .toggleDownload: return "Toggle download"
        case .downloadOnServer: return "Download on server"
        case .removeFromServer: return "Remove from server"
        case .toggleServerDownload: return "Toggle server download"
        }
    }

    var systemImage: String {
        switch self {
        case .none: return "slash.circle"
        case .play: return "play.fill"
        case .stream: return "dot.radiowaves.left.and.right"
        case .togglePlayed: return "checkmark.circle.badge.questionmark"
        case .markPlayed: return "checkmark.circle"
        case .markUnplayed: return "circle"
        case .resetProgress: return "arrow.counterclockwise"
        case .addToQueue: return "text.badge.plus"
        case .removeFromQueue: return "text.badge.minus"
        case .toggleQueue: return "text.badge.checkmark"
        case .removeFromList: return "minus.circle"
        case .addToPlaylist: return "music.note.list"
        case .downloadToDevice: return "arrow.down.to.line.circle"
        case .redownloadDevice: return "arrow.triangle.2.circlepath"
        case .removeDownload: return "iphone.slash"
        case .toggleDownload: return "arrow.down.circle.dotted"
        case .downloadOnServer: return "icloud.and.arrow.down"
        case .removeFromServer: return "icloud.slash"
        case .toggleServerDownload: return "icloud.circle"
        }
    }

    /// Destructive/removal actions get the red tint; additive ones blue-ish.
    var tint: Color {
        switch self {
        case .none: return .gray
        case .play, .stream: return .indigo
        case .togglePlayed, .markPlayed, .markUnplayed, .resetProgress: return .green
        case .addToQueue, .toggleQueue: return .blue
        case .removeFromQueue, .removeFromList: return .red
        case .addToPlaylist: return .purple
        case .downloadToDevice, .redownloadDevice, .toggleDownload,
            .downloadOnServer, .toggleServerDownload:
            return .orange
        case .removeDownload, .removeFromServer: return .red
        }
    }
}

/// The pages whose swipes are configurable — all six of the web's
/// (webui/config swipe.rs `SwipePage::ALL`).
enum SwipePage: String, Codable, CaseIterable, Identifiable {
    case latest
    case queue
    case podcastEpisodes = "podcast_episodes"
    case downloads
    case history
    case playlist

    var id: String { rawValue }

    var label: String {
        switch self {
        case .latest: return "Latest"
        case .queue: return "Queue"
        case .podcastEpisodes: return "Podcast episodes"
        case .downloads: return "Downloads"
        case .history: return "History"
        case .playlist: return "Playlist"
        }
    }

    /// Actions offered for this page, in menu order — `removeFromList` only
    /// where the list IS a playlist (web: `SwipePage::allowed_actions`).
    var allowedActions: [SwipeAction] {
        SwipeAction.allCases.filter { action in
            action != .removeFromList || self == .queue || self == .playlist
        }
    }
}

struct SwipePrefs: Codable, Equatable {
    struct PagePrefs: Codable, Equatable {
        var leading: SwipeAction
        var trailing: SwipeAction
    }

    var pages: [String: PagePrefs]

    /// The web defaults (webui/config swipe.rs): `leading` here is the
    /// web's `left` (fires on a swipe-RIGHT gesture), `trailing` its `right`.
    static let `default` = SwipePrefs(
        pages: [
            SwipePage.latest.rawValue: PagePrefs(
                leading: .addToQueue, trailing: .downloadToDevice),
            SwipePage.queue.rawValue: PagePrefs(
                leading: .removeFromQueue, trailing: .downloadToDevice),
            SwipePage.podcastEpisodes.rawValue: PagePrefs(
                leading: .addToPlaylist, trailing: .downloadToDevice),
            SwipePage.downloads.rawValue: PagePrefs(
                leading: .addToPlaylist, trailing: .redownloadDevice),
            SwipePage.history.rawValue: PagePrefs(
                leading: .addToPlaylist, trailing: .downloadToDevice),
            SwipePage.playlist.rawValue: PagePrefs(
                leading: .removeFromList, trailing: .downloadToDevice),
        ]
    )

    /// Unset pages (saved prefs predating a page) fall back to the web
    /// default, not to a dead swipe.
    func page(_ page: SwipePage) -> PagePrefs {
        pages[page.rawValue]
            ?? Self.default.pages[page.rawValue]
            ?? PagePrefs(leading: .none, trailing: .none)
    }
}

/// Reactive holder + per-account persistence.
@MainActor
@Observable
final class SwipePrefsModel {
    private static let key = "swipe-prefs"

    private(set) var prefs: SwipePrefs = .default
    private let store: LocalStore?

    init(store: LocalStore?) {
        self.store = store
    }

    func load() async {
        if let store, let saved = await store.load(SwipePrefs.self, key: Self.key) {
            prefs = saved
        }
    }

    func update(_ page: SwipePage, leading: SwipeAction, trailing: SwipeAction) {
        prefs.pages[page.rawValue] = SwipePrefs.PagePrefs(leading: leading, trailing: trailing)
        let snapshot = prefs
        Task { [store] in await store?.save(snapshot, key: Self.key) }
    }
}

extension View {
    /// Attach the configured leading/trailing swipe actions for `page`.
    /// `context` resolves the list-scoped actions (remove-from-list) the same
    /// way the row menu does.
    func configuredSwipes(
        _ page: SwipePage, episode: EpisodeData, core: HalogenCore,
        context: EpisodeMenuContext = .browse
    ) -> some View {
        let prefs =
            core.models?.swipes.prefs.page(page)
            ?? SwipePrefs.PagePrefs(leading: .none, trailing: .none)
        return
            self
            .swipeActions(edge: .leading, allowsFullSwipe: true) {
                SwipeActionButton(
                    action: prefs.leading, episode: episode, core: core, context: context)
            }
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                SwipeActionButton(
                    action: prefs.trailing, episode: episode, core: core, context: context)
            }
    }
}

private struct SwipeActionButton: View {
    let action: SwipeAction
    let episode: EpisodeData
    let core: HalogenCore
    var context: EpisodeMenuContext = .browse

    var body: some View {
        if action != .none {
            Button {
                perform()
            } label: {
                Label(action.label, systemImage: action.systemImage)
            }
            .tint(action.tint)
        }
    }

    /// The action→primitive mapping — one place, mirroring the web's
    /// `perform_episode_action` (webui/episode-list action.rs), embedded
    /// device→server remaps included.
    private func perform() {
        guard let models = core.models else { return }
        let embedded = core.isEmbeddedAccount
        switch action {
        case .none:
            break
        case .play:
            models.player.play(episode, context: context.playbackContext)
        case .stream:
            models.player.stream(episode, context: context.playbackContext)
        case .togglePlayed:
            let status = models.playbacks.status(for: episode)
            models.playbacks.markPlayed(episode, played: status != .finished)
        case .markPlayed:
            models.playbacks.markPlayed(episode, played: true)
        case .markUnplayed:
            models.playbacks.markPlayed(episode, played: false)
        case .resetProgress:
            models.playbacks.setCursor(episode, cursor: 0)
        case .addToQueue:
            models.queue.add(episode)
        case .removeFromQueue:
            models.queue.remove(episode)
        case .toggleQueue:
            if models.queue.contains(episode) {
                models.queue.remove(episode)
            } else {
                models.queue.add(episode)
            }
        case .removeFromList:
            switch context {
            case .queue:
                models.queue.remove(episode)
            case .playlist(_, let model):
                model.remove(episode)
            case .browse:
                break  // not offered on browse pages (allowedActions)
            }
        case .addToPlaylist:
            models.playlists.requestPick([episode])
        case .downloadToDevice:
            if embedded {
                models.serverDownloads.download(episode)
            } else {
                models.device.download(episode)
            }
        case .redownloadDevice:
            if embedded {
                redownloadServer()
            } else if !core.isOffline {
                // Web rule: never destroy a local copy you can't re-pull.
                models.device.remove(episode.id)
                models.device.download(episode)
            } else {
                // The refusal is right; the silence wasn't — the swipe just
                // animated closed with no effect.
                ToastCenter.shared.error("You're offline — re-download needs a connection.")
            }
        case .removeDownload:
            if embedded {
                removeServer()
            } else {
                models.device.remove(episode.id)
            }
        case .toggleDownload:
            if embedded {
                toggleServer()
            } else if models.device.state(of: episode.id) == .downloaded {
                models.device.remove(episode.id)
            } else {
                models.device.download(episode)
            }
        case .downloadOnServer:
            models.serverDownloads.download(episode)
        case .removeFromServer:
            removeServer()
        case .toggleServerDownload:
            toggleServer()
        }
    }

    private func removeServer() {
        core.enqueueMutation(.removeServerDownload(episodeId: episode.id)) {
            core.models?.serverDownloads.markRemovedLocally(episode.id)
        }
    }

    /// Remove-then-trigger, in outbox order (web: RedownloadOnServer).
    private func redownloadServer() {
        Task {
            guard
                await core.ensureQueuedBatch([
                    .removeServerDownload(episodeId: episode.id),
                    .triggerDownload(episodeId: episode.id),
                ])
            else { return }
            core.models?.serverDownloads.watch(episode.id)
        }
    }

    private func toggleServer() {
        if core.models?.serverDownloads.isDownloaded(episode) == true {
            removeServer()
        } else {
            core.models?.serverDownloads.download(episode)
        }
    }
}
