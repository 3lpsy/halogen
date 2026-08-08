import SwiftUI

/// The full player: "Up next" preview up top, artwork floating in the middle,
/// then title, scrubber (with chapter ticks), transport with previous/next
/// episode, chapters menu, rate, sleep timer and auto-advance quick toggle
/// anchored to the bottom — the web's NowPlayingScreen surface.
struct PlayerSheet: View {
    @Bindable var player: PlayerModel
    let core: HalogenCore

    @State private var scrubbing = false
    @State private var scrubValue: Double = 0

    var body: some View {
        VStack(spacing: 24) {
            Capsule()
                .fill(.tertiary)
                .frame(width: 36, height: 5)
                .padding(.top, 10)

            if let episode = player.current {
                if let next = player.upNext {
                    upNextRow(next)
                }

                Spacer(minLength: 0)

                Artwork(url: core.episodeArtURL(episode, small: false), size: 260)

                Spacer(minLength: 0)

                VStack(spacing: 4) {
                    Text(episode.title)
                        .font(.headline)
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                        .accessibilityIdentifier("player-title")
                    if let podcast = episode.podcast?.title {
                        Text(podcast).font(.subheadline).foregroundStyle(.secondary)
                    }
                    if let failure = player.failureMessage {
                        // Tappable — retry routes through the full source
                        // rebuild (toggle() handles the failed-item case).
                        Button {
                            player.toggle()
                        } label: {
                            Label(failure, systemImage: "arrow.clockwise.circle.fill")
                                .font(.caption)
                                .foregroundStyle(.orange)
                                .multilineTextAlignment(.center)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("player-retry")
                    } else if player.preparing {
                        HStack(spacing: 6) {
                            ProgressView().controlSize(.small)
                            Text(player.preparingLabel)
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    } else if player.streaming {
                        Label("Streaming", systemImage: "dot.radiowaves.left.and.right")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 24)

                scrubber

                transportRow

                controlsRow
            } else {
                Spacer()
            }
        }
        .padding(.bottom, 12)
        .presentationDetents([.large])
    }

    /// "Up next" preview — the continuation neighbor (queue, or the playlist
    /// played from), the web's top-right UpNext surface. Tap skips to it.
    private func upNextRow(_ next: EpisodeData) -> some View {
        Button {
            player.playNextEpisode()
        } label: {
            HStack(spacing: 10) {
                Artwork(url: core.episodeArtURL(next), size: 36)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Up next")
                        .font(.caption2.weight(.semibold))
                        .textCase(.uppercase)
                        .foregroundStyle(.secondary)
                    Text(next.title)
                        .font(.footnote.weight(.medium))
                        .lineLimit(1)
                    if let podcast = next.podcast?.title {
                        Text(podcast).font(.caption2).foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                Image(systemName: "forward.end.fill")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemFill)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("player-up-next")
        .padding(.horizontal, 24)
    }

    private var transportRow: some View {
        HStack(spacing: 32) {
            Button {
                player.playPreviousEpisode()
            } label: {
                Image(systemName: "backward.end.fill").font(.title2)
            }
            .disabled(!player.hasPrevious)
            Button {
                player.skip(-player.skipBackSecs)
            } label: {
                Image(systemName: "gobackward").font(.title)
            }
            Button {
                player.toggle()
            } label: {
                if player.buffering {
                    // Rebuffering after a dry buffer — a play/pause
                    // glyph over silence misreads as "playing fine".
                    ProgressView()
                        .controlSize(.large)
                        .frame(width: 64, height: 64)
                } else {
                    Image(
                        systemName: player.isPlaying
                            ? "pause.circle.fill" : "play.circle.fill"
                    )
                    .font(.system(size: 64))
                }
            }
            Button {
                player.skip(player.skipForwardSecs)
            } label: {
                Image(systemName: "goforward").font(.title)
            }
            Button {
                player.playNextEpisode()
            } label: {
                Image(systemName: "forward.end.fill").font(.title2)
            }
            .accessibilityIdentifier("player-next")
            .disabled(!player.hasNext)
        }
        .buttonStyle(.plain)
    }

    private var controlsRow: some View {
        HStack(spacing: 16) {
            if !player.chapters.isEmpty {
                chaptersMenu
            }

            Menu {
                ForEach(ClientPrefs.playbackRates, id: \.self) { rate in
                    Button(String(format: "%g×", rate)) {
                        player.setRate(rate)
                    }
                }
            } label: {
                Text(String(format: "%g×", player.rate))
                    .font(.callout.weight(.semibold))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(Color(.secondarySystemFill)))
            }

            // Auto-advance quick toggle — same pref as Settings
            // (web: the now-playing "A" toggle persists immediately).
            Button {
                core.models?.prefs.update { $0.autoAdvance.toggle() }
            } label: {
                Image(
                    systemName: autoAdvance
                        ? "a.circle.fill" : "a.circle"
                )
                .font(.title3)
                .foregroundStyle(autoAdvance ? Color.accentColor : .secondary)
                .padding(.vertical, 6)
            }

            Menu {
                Button("Off") { player.setSleepTimer(minutes: nil) }
                ForEach(sleepMenuMinutes, id: \.self) { m in
                    Button("\(m) min") { player.setSleepTimer(minutes: m) }
                }
                Button("End of episode") { player.setSleepTimer(minutes: 0) }
            } label: {
                Label(sleepLabel, systemImage: "moon.zzz")
                    .font(.callout.weight(.semibold))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(Color(.secondarySystemFill)))
            }
        }
    }

    private var autoAdvance: Bool {
        core.models?.prefs.prefs.autoAdvance ?? true
    }

    /// The quick sleep durations, always including the configured default
    /// (Settings → Playback) so it's armable from here.
    private var sleepMenuMinutes: [Int] {
        let preset = core.models?.prefs.prefs.defaultSleepMinutes ?? 30
        return Array(Set([5, 15, 30, 60, preset])).sorted()
    }

    /// Chapter picker — selecting a marker seeks to it (web: the drop-up
    /// chapters menu). The active marker is checked.
    private var chaptersMenu: some View {
        Menu {
            ForEach(Array(player.chapters.enumerated()), id: \.element.id) { index, chapter in
                Button {
                    player.seek(to: Double(chapter.starts_at_secs))
                } label: {
                    if player.activeChapterIndex == index {
                        Label(
                            "\(Self.time(Double(chapter.starts_at_secs)))  \(chapter.title)",
                            systemImage: "checkmark")
                    } else {
                        Text("\(Self.time(Double(chapter.starts_at_secs)))  \(chapter.title)")
                    }
                }
            }
        } label: {
            Image(systemName: "list.bullet")
                .font(.title3)
                .padding(.vertical, 6)
        }
    }

    private var scrubber: some View {
        VStack(spacing: 4) {
            // Current chapter — updates as playback crosses each marker
            // (web: the title above the seek bar).
            if let idx = player.activeChapterIndex {
                Text("\(idx + 1). \(player.chapters[idx].title)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Slider(
                value: Binding(
                    get: { scrubbing ? scrubValue : player.position },
                    set: { scrubValue = $0 }
                ),
                in: 0...max(player.duration, 1),
                onEditingChanged: { editing in
                    if editing {
                        scrubValue = player.position
                        scrubbing = true
                    } else {
                        scrubbing = false
                        player.seek(to: scrubValue)
                    }
                }
            )
            .overlay {
                // Chapter tick marks — decorative (never intercept touches),
                // the web's absolute-positioned lines over the range input.
                if player.duration > 0, player.chapters.count > 1 {
                    GeometryReader { geo in
                        ForEach(player.chapters, id: \.id) { chapter in
                            let frac = Double(chapter.starts_at_secs) / player.duration
                            if frac > 0, frac < 1 {
                                Rectangle()
                                    .fill(.secondary.opacity(0.6))
                                    .frame(width: 1, height: 8)
                                    .position(
                                        x: geo.size.width * frac,
                                        y: geo.size.height / 2)
                            }
                        }
                    }
                    .allowsHitTesting(false)
                }
            }
            HStack {
                Text(Self.time(scrubbing ? scrubValue : player.position))
                Spacer()
                Text("-" + Self.time(max(0, player.duration - (scrubbing ? scrubValue : player.position))))
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 24)
    }

    private var sleepLabel: String {
        if player.sleepAtEpisodeEnd { return "Sleep: end" }
        if let mins = player.sleepRemainingMinutes {
            return "Sleep: \(mins)m"
        }
        return "Sleep"
    }

    private static func time(_ seconds: Double) -> String {
        let s = Int(seconds)
        return s >= 3600
            ? String(format: "%d:%02d:%02d", s / 3600, (s % 3600) / 60, s % 60)
            : String(format: "%d:%02d", s / 60, s % 60)
    }
}
