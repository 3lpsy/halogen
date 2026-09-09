import SwiftUI

/// The persistent mini player above the dock (visible on every tab while
/// something is loaded). Tap opens the full player sheet.
struct MiniPlayerBar: View {
    @Bindable var player: PlayerModel
    let core: HalogenCore

    @State private var showSheet = false

    private func deviceProgress(_ episode: EpisodeData) -> Double? {
        if case .downloading(let p) = core.models?.device.state(of: episode.id) ?? .none {
            return p
        }
        return nil
    }

    var body: some View {
        // The sheet is anchored on the Group, OUTSIDE the `if let`: when the
        // bar unmounts on `current == nil`, an inside-anchored sheet loses
        // its presenter and iOS force-dismisses it (racy on slow machines).
        // Terminal stops close it deliberately via onChange below.
        Group {
            if let episode = player.current {
                bar(episode)
            }
        }
        .sheet(isPresented: $showSheet) {
            PlayerSheet(player: player, core: core)
        }
        .onChange(of: player.current == nil) { _, isNil in
            if isNil { showSheet = false }
        }
    }

    @ViewBuilder
    private func bar(_ episode: EpisodeData) -> some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) {
                Artwork(url: core.episodeArtURL(episode), size: 36)
                VStack(alignment: .leading, spacing: 1) {
                    Text(episode.title)
                        .font(.footnote.weight(.medium))
                        .lineLimit(1)
                        .accessibilityIdentifier("mini-title")
                    if let failure = player.failureMessage {
                        Text(failure).font(.caption2).foregroundStyle(.orange)
                            .lineLimit(1)
                    } else if player.preparing {
                        Text(player.preparingLabel).font(.caption2)
                            .foregroundStyle(.secondary)
                    } else if player.streaming {
                        HStack(spacing: 4) {
                            Image(systemName: "dot.radiowaves.left.and.right")
                            Text("Streaming")
                            if let podcast = episode.podcast?.title {
                                Text("· \(podcast)").lineLimit(1)
                            }
                        }
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    } else if let podcast = episode.podcast?.title {
                        Text(podcast).font(.caption2).foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                if player.preparing {
                    HStack(spacing: 6) {
                        ProgressView()
                        if let p = player.current.flatMap({ episode in
                            core.models?.serverDownloads.progress(of: episode.id)
                                ?? deviceProgress(episode)
                        }) {
                            Text("\(Int(p * 100))%")
                                .font(.caption2.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                    }
                } else if let failure = player.failureMessage {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help(failure)
                }
                Button {
                    if player.failureMessage != nil, let episode = player.current {
                        player.play(episode)
                    } else {
                        player.toggle()
                    }
                } label: {
                    if player.buffering {
                        ProgressView().frame(width: 20)
                    } else {
                        Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                            .font(.title3)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("mini-play")
                Button {
                    player.skip(player.skipForwardSecs)
                } label: {
                    Image(systemName: "goforward")
                        .font(.title3)
                }
                .buttonStyle(.plain)
                // Close/stop — without it the bar is undismissable for
                // the whole session (web's mini player has stop).
                Button {
                    player.stop()
                } label: {
                    Image(systemName: "xmark")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 22, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("mini-stop")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            // Thin position track along the bar's bottom edge.
            if player.duration > 0 {
                GeometryReader { geo in
                    Rectangle()
                        .fill(Color.accentColor)
                        .frame(
                            width: geo.size.width
                                * min(max(player.position / player.duration, 0), 1))
                }
                .frame(height: 2)
            }
        }
        .background(.bar)
        .contentShape(Rectangle())
        .onTapGesture { showSheet = true }
        // No container identifier: SwiftUI propagates it onto every
        // child, clobbering mini-play's own identifier.
    }
}
