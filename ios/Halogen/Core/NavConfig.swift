import Foundation
import Observation

/// Built-in navigation destinations — mirrors the web's `BuiltinNav`
/// (crates/ui-config nav.rs), same snake_case tokens so a future config sync
/// could share them.
enum BuiltinNav: String, Codable, CaseIterable, Identifiable {
    case queue
    case latest
    case podcasts
    case playlists
    case downloads
    case discover
    case history
    case settings
    case polling
    case deviceLogs = "device_logs"
    case serverLogs = "server_logs"

    var id: String { rawValue }

    var label: String {
        switch self {
        case .queue: return "Queue"
        case .latest: return "Latest"
        case .podcasts: return "Podcasts"
        case .playlists: return "Playlists"
        case .downloads: return "Downloads"
        case .discover: return "Discover"
        case .history: return "History"
        case .settings: return "Settings"
        case .polling: return "Polling"
        case .deviceLogs: return "Device Logs"
        case .serverLogs: return "Server Logs"
        }
    }

    /// Admin-only destinations (web: `NavItem.admin_only` — Polling and
    /// Server Logs are dropped from every nav surface for non-admins).
    var adminOnly: Bool {
        switch self {
        case .polling, .serverLogs: return true
        default: return false
        }
    }

    var systemImage: String {
        switch self {
        case .queue: return "list.bullet"
        case .latest: return "clock"
        case .podcasts: return "square.grid.2x2"
        case .playlists: return "music.note.list"
        case .downloads: return "arrow.down.circle"
        case .discover: return "magnifyingglass"
        case .history: return "clock.arrow.circlepath"
        case .settings: return "gear"
        case .polling: return "arrow.triangle.2.circlepath"
        case .deviceLogs: return "doc.text"
        case .serverLogs: return "server.rack"
        }
    }
}

/// Nav order/visibility — the web's `NavConfig` shape (order + hidden +
/// pinned playlist ids), persisted per account. The dock renders the first
/// `dockSlots` visible items plus the always-present More tab; the More menu
/// renders everything visible.
struct NavConfig: Codable, Equatable {
    var order: [BuiltinNav]
    var hidden: [BuiltinNav]
    /// Playlist ids pinned as nav links (parity field; pin UI not built yet).
    var pinnedPlaylists: [Int32]

    static let `default` = NavConfig(
        order: [
            .queue, .latest, .podcasts, .playlists, .downloads,
            .discover, .history, .settings, .polling, .deviceLogs, .serverLogs,
        ],
        hidden: [.polling, .deviceLogs, .serverLogs],
        pinnedPlaylists: []
    )

    /// Self-heal persisted orders that predate newly added builtins.
    mutating func normalize() {
        for key in Self.default.order where !order.contains(key) {
            order.append(key)
        }
        // Settings can never be hidden (same hard rule as the web).
        hidden.removeAll { $0 == .settings }
    }

    var visible: [BuiltinNav] {
        order.filter { !hidden.contains($0) }
    }
}

/// Reactive holder + persistence for the account's NavConfig.
@MainActor
@Observable
final class NavModel {
    /// iPhone shows at most 5 tab slots and the More tab always takes one, so
    /// 4 destinations ride the dock (the web dock fits 5 + More; platform
    /// divergence documented in docs/internal/IOS_NATIVE_SWIFT.md).
    static let dockSlots = 4
    private static let key = "nav-config"

    private(set) var config: NavConfig = .default
    private let store: LocalStore?

    init(store: LocalStore?) {
        self.store = store
    }

    func load() async {
        if let store, var saved = await store.load(NavConfig.self, key: Self.key) {
            saved.normalize()
            config = saved
        }
    }

    func update(_ new: NavConfig) {
        var normalized = new
        normalized.normalize()
        config = normalized
        let snapshot = normalized
        Task { [store] in await store?.save(snapshot, key: Self.key) }
    }

    /// Every destination this user may see, in configured order — the web's
    /// `nav_items`: hidden dropped, admin-only dropped for non-admins.
    func visibleItems(isAdmin: Bool) -> [BuiltinNav] {
        config.visible.filter { isAdmin || !$0.adminOnly }
    }

    /// The tab bar: first N visible destinations (More is appended by the UI).
    func dockItems(isAdmin: Bool) -> [BuiltinNav] {
        Array(visibleItems(isAdmin: isAdmin).prefix(Self.dockSlots))
    }
}
