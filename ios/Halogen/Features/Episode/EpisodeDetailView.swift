import SwiftUI

/// Episode detail: full metadata, actions (play / queue / download / played),
/// description, chapters. Local-first — the fetched episode caches per id, so
/// anything opened once re-renders offline.
struct EpisodeDetailView: View {
    let core: HalogenCore
    let episodeId: Int32

    @State private var model: EpisodeDetailModel
    @Environment(Navigator.self) private var navigator: Navigator?

    init(core: HalogenCore, episodeId: Int32) {
        self.core = core
        self.episodeId = episodeId
        _model = State(initialValue: EpisodeDetailModel(core: core, episodeId: episodeId))
    }

    var body: some View {
        Group {
            if let episode = model.episode {
                content(episode)
            } else if let error = model.error {
                ContentUnavailableView {
                    Label("Couldn't load episode", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error).font(.footnote.monospaced())
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Episode")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                // The web's detail kebab: the SAME shared menu the list rows
                // use (stream / downloads / playlist / queue / played /
                // purge), plus View metadata.
                Menu {
                    if let episode = model.episode {
                        EpisodeMenu(episode: episode, context: .browse, core: core)
                        Divider()
                    }
                    Button {
                        // A Navigator push — NavigationLinks inside Menus
                        // never fire (see PodcastManage).
                        navigator?.push(.episodeMetadata(episodeId))
                    } label: {
                        Label("View metadata", systemImage: "info.circle")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
        .task { await model.load() }
        // A row already DOWNLOADING server-side gets live tracking (the
        // web's durable-status poll) …
        .task(id: model.episode?.download_status) {
            if model.episode?.download_status == .downloading {
                core.models?.serverDownloads.watch(episodeId)
            }
        }
        // … and the tracked outcome (success OR failure) refreshes the
        // snapshot, so the page never stays "downloading" until a manual
        // refresh.
        .onChange(of: serverOutcome) { _, done in
            if done { Task { await model.refresh() } }
        }
        .refreshable { await model.refresh() }
    }

    private var serverOutcome: Bool {
        core.models?.serverDownloads.completed.contains(episodeId) == true
            || core.models?.serverDownloads.failed.contains(episodeId) == true
    }

    private func content(_ episode: EpisodeData) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .top, spacing: 14) {
                    Artwork(url: core.episodeArtURL(episode, small: false), size: 96)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(episode.title).font(.headline)
                        if let podcast = episode.podcast {
                            Button {
                                navigator?.push(.podcast(podcast.id))
                            } label: {
                                Text(podcast.title)
                                    .font(.subheadline)
                                    .foregroundStyle(Color.accentColor)
                            }
                            .buttonStyle(.plain)
                        }
                        metaLine(episode)
                    }
                }

                actions(episode)

                // Playback-position bar (overlay-wins cursor / duration) —
                // the web detail's progress_pct track.
                if let progress = playbackProgress(episode) {
                    ProgressView(value: progress)
                        .progressViewStyle(.linear)
                        .tint(Color.accentColor)
                }

                if let description = episode.description, !description.isEmpty {
                    Divider()
                    HTMLDescription(html: description)
                        .font(.callout)
                        .foregroundStyle(.primary)
                        .tint(Color.accentColor)
                }

                if let chapters = episode.chapters, !chapters.isEmpty {
                    Divider()
                    Text("Chapters").font(.headline)
                    ForEach(chapters, id: \.id) { chapter in
                        HStack {
                            Text(Self.timestamp(chapter.starts_at_secs))
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .frame(width: 56, alignment: .leading)
                            Text(chapter.title).font(.callout)
                        }
                    }
                }
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func metaLine(_ episode: EpisodeData) -> some View {
        HStack(spacing: 6) {
            if let published = episode.published_at {
                Text(published, format: .dateTime.day().month().year())
            }
            if let secs = episode.duration_secs, secs > 0 {
                Text("·")
                Text("\(Int(secs) / 60) min")
            }
            if playedStatus(episode) == .finished {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private func actions(_ episode: EpisodeData) -> some View {
        HStack(spacing: 12) {
            primaryButton(episode)

            if let queue = core.models?.queue {
                Button {
                    queue.contains(episode) ? queue.remove(episode) : queue.add(episode)
                } label: {
                    Image(
                        systemName: queue.contains(episode)
                            ? "text.badge.minus" : "text.badge.plus")
                }
                .buttonStyle(.bordered)
            }

            Button {
                Task { await model.togglePlayed() }
            } label: {
                Image(
                    systemName: playedStatus(episode) == .finished
                        ? "checkmark.circle.fill" : "checkmark.circle")
            }
            .buttonStyle(.bordered)

            // The tiered download control the list rows use (device trash /
            // device+server progress rings / cloud pull / plain download) —
            // one shared component so the tiers can't drift.
            DownloadButton(episode: episode, core: core)
                .padding(.horizontal, 4)
        }
    }

    /// The web detail's primary button: "Download & Play" when the episode
    /// is nowhere (plain "Download" on embedded), otherwise Play/Pause with
    /// in-button download progress while a copy is being fetched.
    @ViewBuilder
    private func primaryButton(_ episode: EpisodeData) -> some View {
        let device =
            core.isEmbeddedAccount
            ? DeviceDownloads.State.none
            : (core.models?.device.state(of: episode.id) ?? .none)
        let onServer = core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)
        let serverProgress = core.models?.serverDownloads.progress(of: episode.id)
        let serverRunning =
            serverProgress != nil
            || (episode.download_status == .downloading
                && core.models?.serverDownloads.completed.contains(episode.id) != true
                && core.models?.serverDownloads.failed.contains(episode.id) != true)
        let deviceProgress: Double? = {
            if case .downloading(let p) = device { return p }
            return nil
        }()
        let isCurrent = core.models?.player.current?.id == episode.id
        let preparing = isCurrent && core.models?.player.preparing == true
        let nowhere =
            !onServer && device != .downloaded && !serverRunning && deviceProgress == nil
            && !preparing

        if nowhere {
            if core.isEmbeddedAccount {
                // Embedded: a plain server download — Play then streams from
                // the built-in server (web: the embedded Download branch).
                Button {
                    core.models?.serverDownloads.download(episode)
                } label: {
                    Label("Download", systemImage: "arrow.down.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(core.isOffline)
            } else {
                // Force fetch→play with progress (the player's preparation
                // pipeline mirrors the strategy, captions included).
                Button {
                    // Web episode_detail: download_and_play_in(id, None) —
                    // playing from the detail resets to queue semantics.
                    core.models?.player.play(episode, context: nil)
                } label: {
                    Label("Download & Play", systemImage: "icloud.and.arrow.down")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(core.isOffline)
            }
        } else {
            Button {
                if isCurrent {
                    core.models?.player.toggle()
                } else {
                    // Web episode_detail: request_play_in(id, None).
                    core.models?.player.play(episode, context: nil)
                }
            } label: {
                HStack(spacing: 6) {
                    if preparing {
                        ProgressView()
                        Text("Preparing…")
                    } else if let pct = deviceProgress {
                        ProgressView()
                        Text("To device… \(Int(pct * 100))%")
                    } else if serverRunning {
                        ProgressView()
                        if core.isEmbeddedAccount {
                            Text(serverPercent(serverProgress, prefix: "Downloading…"))
                        } else {
                            Text(serverPercent(serverProgress, prefix: "On server…"))
                        }
                    } else if isCurrent, core.models?.player.isPlaying == true {
                        Label("Pause", systemImage: "pause.fill")
                    } else {
                        Label("Play", systemImage: "play.fill")
                    }
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .disabled(preparing)
        }
    }

    private func serverPercent(_ progress: Double?, prefix: String) -> String {
        guard let progress else { return prefix }
        return "\(prefix) \(Int(progress * 100))%"
    }

    /// Overlay-wins played facet (an offline toggle flips this immediately).
    private func playedStatus(_ episode: EpisodeData) -> PlaybackStatus {
        core.models?.playbacks.status(for: episode)
            ?? episode.playback_status ?? .unplayed
    }

    /// Overlay-wins position fraction for the progress bar.
    private func playbackProgress(_ episode: EpisodeData) -> Double? {
        guard let duration = episode.duration_secs, duration > 0 else { return nil }
        let cursor = core.models?.playbacks.cursor(for: episode) ?? episode.playback?.cursor
        guard let cursor, cursor > 0 else { return nil }
        return min(Double(cursor) / Double(duration), 1)
    }

    private static func timestamp(_ secs: Int32) -> String {
        let s = Int(secs)
        return s >= 3600
            ? String(format: "%d:%02d:%02d", s / 3600, (s % 3600) / 60, s % 60)
            : String(format: "%d:%02d", s / 60, s % 60)
    }
}

