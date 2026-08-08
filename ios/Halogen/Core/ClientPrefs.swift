import Foundation
import Observation
import SwiftUI

/// Per-account client preferences — the native ClientConfig: UI size/theme,
/// playback behavior, and device-download chunking. Persisted per account.
struct ClientPrefs: Codable, Equatable {
    enum UISize: String, Codable, CaseIterable, Identifiable {
        case small, medium, large, xlarge

        var id: String { rawValue }

        var label: String {
            switch self {
            case .small: return "Small (100%)"
            case .medium: return "Medium (125%, default)"
            case .large: return "Large (135%)"
            case .xlarge: return "XLarge (165%)"
            }
        }

        /// Mapping onto Dynamic Type — the web's ladder (Small 100% /
        /// Medium 125% default / larger steps up): medium renders ~25%
        /// bigger than the system default (body 21pt vs 17pt). Small IS the
        /// system default, not below it.
        var dynamicType: DynamicTypeSize {
            switch self {
            case .small: return .large
            case .medium: return .xxLarge
            case .large: return .xxxLarge
            case .xlarge: return .accessibility1
            }
        }
    }

    enum AppTheme: String, Codable, CaseIterable, Identifiable {
        case dark, light

        var id: String { rawValue }
        var label: String { rawValue.capitalized }

        var colorScheme: ColorScheme {
            self == .dark ? .dark : .light
        }
    }

    /// How the play button sources audio — the web's PlaybackPreference,
    /// same tokens. Embedded accounts are forced to streamOnly (the media
    /// already lives on this device inside the server; duplicating it into
    /// the client store would be waste).
    enum PlaybackStrategy: String, Codable, CaseIterable, Identifiable {
        case downloadOnly = "DownloadOnly"
        case streamFirstAndDownload = "StreamFirstAndDownload"
        case streamFallback = "StreamFallback"
        case streamOnly = "StreamOnly"

        var id: String { rawValue }

        var label: String {
            switch self {
            case .downloadOnly: return "Download only (local-first)"
            case .streamFirstAndDownload: return "Stream first, download in background"
            case .streamFallback: return "Local first, stream as fallback"
            case .streamOnly: return "Stream only"
            }
        }
    }

    var uiSize: UISize
    var theme: AppTheme
    /// Seconds for the player's skip buttons + lock-screen commands.
    var playbackStrategy: PlaybackStrategy
    var skipForwardSecs: Int
    var skipBackSecs: Int
    var defaultRate: Float
    /// Auto-play the next queue/playlist item when an episode ends — the
    /// web's PlaybackPrefs.auto_advance (default true).
    var autoAdvance: Bool
    /// Single "Add to Queue" inserts at the FRONT of the queue (newest first)
    /// — the web's add_to_queue_front (default true).
    var addToQueueFront: Bool
    /// Minutes the sleep timer arms with (tap or auto-arm) — the web's
    /// default_sleep_minutes.
    var defaultSleepMinutes: Int
    /// Auto-arm the sleep timer once per listening session — the web's
    /// sleep_by_default.
    var sleepByDefault: Bool
    /// Device-download chunk size (KiB) — each chunk is an independent
    /// resumable unit. `0` = no chunking (one streamed whole-file request).
    /// Mirrors the web's DownloadChunkSize (2–32 MB + NoChunking, default 4 MB).
    var downloadChunkKiB: Int
    /// Concurrent chunk fetches WITHIN one download (writes still land in
    /// byte order) — the web's DownloadPrefs.parallelism (default 1 =
    /// sequential). Ignored under no-chunking (a single request).
    var downloadParallelism: Int
    /// Discover providers the user toggled OFF (raw wire tokens) — the web's
    /// ClientConfig.disabled_discover_providers.
    var disabledDiscoverProviders: [String]
    /// Bluetooth/lock-screen next/previous-track actions SEEK within the
    /// episode instead of switching episodes — the web's
    /// PlaybackPrefs.media_next_prev_seek (default false).
    var mediaNextPrevSeek: Bool

    /// The web's PLAYBACK_RATES — shared by the settings picker and the
    /// player's speed menu so both offer the same set.
    static let playbackRates: [Float] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]
    /// The web's SLEEP_DURATIONS (minutes) for the settings picker.
    static let sleepDurations: [Int] = [5, 10, 15, 20, 30, 45, 60, 90, 120]
    /// The web's DownloadChunkSize options in KiB (2–32 MB; 0 = no chunking).
    static let downloadChunkKiBOptions: [Int] = [2048, 4096, 8192, 16384, 32768, 0]
    /// The web's DOWNLOAD_PARALLELISMS.
    static let downloadParallelisms: [Int] = [1, 2, 4, 8]

    static let `default` = ClientPrefs(
        uiSize: .medium,
        theme: .dark,
        playbackStrategy: .downloadOnly,
        skipForwardSecs: 30,
        skipBackSecs: 15,
        defaultRate: 1.0,
        autoAdvance: true,
        addToQueueFront: true,
        defaultSleepMinutes: 30,
        sleepByDefault: false,
        downloadChunkKiB: 4096,
        downloadParallelism: 1,
        disabledDiscoverProviders: [],
        mediaNextPrevSeek: false
    )

    // Tolerant decode so new fields never wipe stored prefs.
    init(
        uiSize: UISize, theme: AppTheme, playbackStrategy: PlaybackStrategy,
        skipForwardSecs: Int, skipBackSecs: Int,
        defaultRate: Float, autoAdvance: Bool, addToQueueFront: Bool,
        defaultSleepMinutes: Int, sleepByDefault: Bool,
        downloadChunkKiB: Int, downloadParallelism: Int,
        disabledDiscoverProviders: [String],
        mediaNextPrevSeek: Bool
    ) {
        self.uiSize = uiSize
        self.theme = theme
        self.playbackStrategy = playbackStrategy
        self.skipForwardSecs = skipForwardSecs
        self.skipBackSecs = skipBackSecs
        self.defaultRate = defaultRate
        self.autoAdvance = autoAdvance
        self.addToQueueFront = addToQueueFront
        self.defaultSleepMinutes = defaultSleepMinutes
        self.sleepByDefault = sleepByDefault
        self.downloadChunkKiB = downloadChunkKiB
        self.downloadParallelism = downloadParallelism
        self.disabledDiscoverProviders = disabledDiscoverProviders
        self.mediaNextPrevSeek = mediaNextPrevSeek
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = Self.default
        uiSize = (try? c.decode(UISize.self, forKey: .uiSize)) ?? d.uiSize
        theme = (try? c.decode(AppTheme.self, forKey: .theme)) ?? d.theme
        playbackStrategy =
            (try? c.decode(PlaybackStrategy.self, forKey: .playbackStrategy))
            ?? d.playbackStrategy
        skipForwardSecs = (try? c.decode(Int.self, forKey: .skipForwardSecs)) ?? d.skipForwardSecs
        skipBackSecs = (try? c.decode(Int.self, forKey: .skipBackSecs)) ?? d.skipBackSecs
        defaultRate = (try? c.decode(Float.self, forKey: .defaultRate)) ?? d.defaultRate
        autoAdvance = (try? c.decode(Bool.self, forKey: .autoAdvance)) ?? d.autoAdvance
        addToQueueFront =
            (try? c.decode(Bool.self, forKey: .addToQueueFront)) ?? d.addToQueueFront
        defaultSleepMinutes =
            (try? c.decode(Int.self, forKey: .defaultSleepMinutes)) ?? d.defaultSleepMinutes
        sleepByDefault = (try? c.decode(Bool.self, forKey: .sleepByDefault)) ?? d.sleepByDefault
        // Snap stored values onto the web-parity option sets — pre-parity
        // builds persisted 256/512/1024 KiB chunk sizes that are no longer
        // offered (an out-of-set value falls back to the default, the web's
        // from_str_or_default behavior).
        let storedChunk =
            (try? c.decode(Int.self, forKey: .downloadChunkKiB)) ?? d.downloadChunkKiB
        downloadChunkKiB =
            Self.downloadChunkKiBOptions.contains(storedChunk) ? storedChunk : d.downloadChunkKiB
        let storedParallelism =
            (try? c.decode(Int.self, forKey: .downloadParallelism)) ?? d.downloadParallelism
        downloadParallelism =
            Self.downloadParallelisms.contains(storedParallelism)
            ? storedParallelism : d.downloadParallelism
        disabledDiscoverProviders =
            (try? c.decode([String].self, forKey: .disabledDiscoverProviders))
            ?? d.disabledDiscoverProviders
        mediaNextPrevSeek =
            (try? c.decode(Bool.self, forKey: .mediaNextPrevSeek)) ?? d.mediaNextPrevSeek
    }
}

/// Reactive holder + per-account persistence.
@MainActor
@Observable
final class ClientPrefsModel {
    private static let key = "client-prefs"

    private(set) var prefs: ClientPrefs = .default
    private let store: LocalStore?

    init(store: LocalStore?) {
        self.store = store
    }

    func load() async {
        if let store, let saved = await store.load(ClientPrefs.self, key: Self.key) {
            prefs = saved
        }
    }

    func update(_ mutate: (inout ClientPrefs) -> Void) {
        mutate(&prefs)
        let snapshot = prefs
        Task { [store] in await store?.save(snapshot, key: Self.key) }
    }
}
