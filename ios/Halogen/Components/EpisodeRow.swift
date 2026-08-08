import SwiftUI

/// Pure display pieces for an episode list row. The full row (with its
/// controls and navigation targets) is `EpisodeRowLink` in EpisodeMenu.swift.
enum EpisodeRowStyle {
    /// The web's PlaybackMarker: rows show at most ONE status icon, by
    /// precedence — next-up first (it's what plays when the current episode
    /// ends), then finished, then in-progress.
    enum PlaybackMarker {
        case none
        case finished
        case inProgress
        case nextUp
    }

    static func duration(_ secs: Int32) -> String {
        // Floor at 1 — a 45-second trailer must not read "0 min" (web parity).
        let mins = max(1, Int(secs) / 60)
        if mins >= 60 {
            return "\(mins / 60) hr \(mins % 60) min"
        }
        return "\(mins) min"
    }
}

/// The single status glyph for a row (nothing for `.none`).
struct PlaybackMarkerIcon: View {
    let marker: EpisodeRowStyle.PlaybackMarker

    var body: some View {
        switch marker {
        case .none:
            EmptyView()
        case .finished:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .inProgress:
            Image(systemName: "circle.lefthalf.filled")
                .foregroundStyle(Color.accentColor)
        case .nextUp:
            Image(systemName: "arrow.forward.circle")
                .foregroundStyle(Color.accentColor)
        }
    }
}
