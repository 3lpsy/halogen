import Foundation

/// One multi-select episode filter chip (web `EpisodeFilter` vocabulary): OR
/// within a facet, AND across facets. `onDevice` is client-only (never
/// expressible server-side); the rest map onto wire `FilterParams` tokens when
/// a facet has exactly one chip, and are enforced locally otherwise.
enum EpisodeFilter: String, Codable, CaseIterable, Identifiable {
    case onDevice = "ondevice"
    case downloaded
    case downloading
    case unplayed
    case inProgress = "played"
    case finished

    var id: String { rawValue }

    var label: String {
        switch self {
        case .onDevice: return "On Device"
        case .downloaded: return "Downloaded"
        case .downloading: return "Downloading"
        case .unplayed: return "Unplayed"
        case .inProgress: return "In Progress"
        case .finished: return "Finished"
        }
    }

    /// The download facet (server download state).
    static let downloadFacet: [EpisodeFilter] = [.downloaded, .downloading]
    /// The played-state facet (the 3-state playback_status column).
    static let playedFacet: [EpisodeFilter] = [.unplayed, .inProgress, .finished]

    /// The wire `FilterParams` token for a single-chip facet selection
    /// (values are the serde-renamed enum tokens).
    var wireToken: String? {
        switch self {
        case .onDevice: return nil
        case .downloaded: return "DOWNLOADED"
        case .downloading: return "DOWNLOADING"
        case .unplayed: return "UNPLAYED"
        case .inProgress: return "PLAYED"
        case .finished: return "FINISHED"
        }
    }
}
