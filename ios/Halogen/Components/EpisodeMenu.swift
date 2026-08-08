import SwiftUI

/// Where an episode row is rendered — decides which quick actions its menu
/// offers (the web's per-page row context menus).
enum EpisodeMenuContext {
    /// Browsing lists (Latest / podcast episodes / History / Downloads).
    case browse
    /// The queue: adds move-to-top + remove-from-queue.
    case queue
    /// A playlist detail: adds remove-from-<name>. Carries the screen's model
    /// so the remove updates the visible list + cache (same path as the swipe).
    case playlist(name: String, model: PlaylistEpisodesModel)

    /// The play context this row's list contributes (web request_play_in): a
    /// playlist detail continues through that playlist; browse/queue reset to
    /// queue semantics (nil ≡ the web's None).
    @MainActor
    var playbackContext: PlayerModel.PlaybackContext? {
        if case .playlist(_, let model) = self {
            return PlayerModel.PlaybackContext(
                playlistId: model.playlistId, episodes: model.episodes)
        }
        return nil
    }
}

/// The quick-action menu content for an episode row — shared by the visible
/// ellipsis button and the long-press context menu so both always agree.
struct EpisodeMenu: View {
    let episode: EpisodeData
    let context: EpisodeMenuContext
    let core: HalogenCore

    @Environment(Navigator.self) private var navigator: Navigator?

    var body: some View {
        Button {
            core.models?.player.play(episode, context: context.playbackContext)
        } label: {
            Label("Play", systemImage: "play.fill")
        }
        streamAction

        queueSection
        playlistSection
        playedToggle
        downloadSection
        navigationSection
        contextSection
        recoverySection
    }

    /// "Stream from server" escape hatch — shown when the server holds a copy,
    /// this device doesn't, and the strategy allows streaming (web
    /// episode_menu.rs gates; embedded Play already IS local streaming).
    @ViewBuilder
    private var streamAction: some View {
        let onServer = core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)
        let onDevice = core.models?.device.state(of: episode.id) == .downloaded
        let strategy = core.models?.prefs.prefs.playbackStrategy ?? .downloadOnly
        if !core.isEmbeddedAccount, strategy != .downloadOnly, onServer, !onDevice,
            !core.isOffline
        {
            Button {
                core.models?.player.stream(episode, context: context.playbackContext)
            } label: {
                Label("Stream from server", systemImage: "dot.radiowaves.left.and.right")
            }
        }
    }

    // MARK: - pieces

    @ViewBuilder
    private var queueSection: some View {
        if let queue = core.models?.queue {
            if case .queue = context {
                Button {
                    queue.moveToTop(episode)
                } label: {
                    Label("Move to top", systemImage: "arrow.up.to.line")
                }
            } else if queue.contains(episode) {
                Button {
                    queue.remove(episode)
                } label: {
                    Label("Remove from Queue", systemImage: "minus.circle")
                }
            } else {
                Button {
                    queue.add(episode)
                } label: {
                    Label("Add to Queue", systemImage: "text.badge.plus")
                }
            }
        }
    }

    /// Membership-aware playlist toggles (web episode_playlists.rs): checkmark
    /// when already a member, tap toggles add ↔ remove — optimistic outbox ops
    /// + cache patches, so the menu and target list flip offline too.
    @ViewBuilder
    private var playlistSection: some View {
        if let playlists = core.models?.playlists {
            Menu {
                ForEach(playlists.playlists.filter { !$0.is_default }, id: \.id) { playlist in
                    let member = playlist.episode_ids?.contains(episode.id) == true
                    Button {
                        if member {
                            playlists.remove(episode, from: playlist)
                        } else {
                            playlists.add(episode, to: playlist)
                        }
                    } label: {
                        if member {
                            Label(playlist.name, systemImage: "checkmark")
                        } else {
                            Text(playlist.name)
                        }
                    }
                }
            } label: {
                // Not plain "Playlists" — that label collides with the tab
                // bar button in the accessibility tree (ambiguous UI tests).
                Label("Manage Playlists", systemImage: "music.note.list")
            }
        }
    }

    @ViewBuilder
    private var playedToggle: some View {
        // Overlay-wins: an offline toggle must flip this menu immediately,
        // not keep offering the same action off a stale row snapshot.
        let status = core.models?.playbacks.status(for: episode)
            ?? episode.playback_status ?? .unplayed
        if status == .finished {
            Button {
                core.models?.playbacks.markPlayed(episode, played: false)
            } label: {
                Label("Mark unplayed", systemImage: "circle")
            }
        } else {
            Button {
                core.models?.playbacks.markPlayed(episode, played: true)
            } label: {
                Label("Mark played", systemImage: "checkmark.circle")
            }
        }
    }

    @ViewBuilder
    private var downloadSection: some View {
        deviceSection
        let embedded = core.isEmbeddedAccount
        let onServer = core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)
        if onServer {
            Button {
                // Force fresh: remove drains first, then the trigger
                // re-fetches (web: RedownloadOnServer's op order).
                Task {
                    await core.outbox?.enqueue(.removeServerDownload(episodeId: episode.id))
                    core.models?.serverDownloads.download(episode)
                }
            } label: {
                Label(
                    embedded ? "Re-download" : "Re-download on server",
                    systemImage: "arrow.triangle.2.circlepath")
            }
            Button(role: .destructive) {
                // Optimistic overlay first (rows/menus flip immediately), then
                // the durable op (queues offline instead of silently dropping).
                core.models?.serverDownloads.markRemovedLocally(episode.id)
                Task {
                    await core.outbox?.enqueue(.removeServerDownload(episodeId: episode.id))
                }
            } label: {
                Label(
                    embedded ? "Remove download" : "Remove from server",
                    systemImage: embedded ? "trash" : "icloud.slash")
            }
        } else if episode.download_status != .downloading {
            Button {
                core.models?.serverDownloads.download(episode)
            } label: {
                Label(
                    embedded ? "Download" : "Download on server",
                    systemImage: embedded ? "arrow.down.circle" : "icloud.and.arrow.down")
            }
        }
    }

    /// "View podcast" (web parity). A Button through the Navigator, NOT a
    /// NavigationLink: links inside Menus never fire.
    private var navigationSection: some View {
        Button {
            navigator?.push(.podcast(episode.podcast_id))
        } label: {
            Label("View podcast", systemImage: "square.grid.2x2")
        }
    }

    /// "Remove local data" — wipe THIS episode's local traces without touching
    /// the server; it re-syncs on the next pull (web confirm_purge).
    private var recoverySection: some View {
        Button(role: .destructive) {
            core.models?.device.remove(episode.id)
            core.models?.playbacks.purge(episodeId: episode.id)
            Task { [store = core.store, id = episode.id] in
                await store?.remove(key: CacheKey.episode(id))
            }
            // Menus can't host a confirm dialog; at minimum the destructive
            // wipe must acknowledge itself.
            ToastCenter.shared.success("Removed this episode's local data")
        } label: {
            Label("Remove local data", systemImage: "arrow.counterclockwise")
        }
    }

    @ViewBuilder
    private var deviceSection: some View {
        if let device = core.models?.device, !core.isEmbeddedAccount {
            switch device.state(of: episode.id) {
            case .downloaded, .downloading, .paused, .waitingServer:
                Button(role: .destructive) {
                    device.remove(episode.id)
                } label: {
                    Label("Remove from device", systemImage: "iphone.slash")
                }
            case .failed:
                // Retry (resumes the durable partial) or clear the traces.
                Button {
                    device.download(episode)
                } label: {
                    Label("Retry download to device", systemImage: "arrow.down.to.line.circle")
                }
                Button(role: .destructive) {
                    device.remove(episode.id)
                } label: {
                    Label("Remove from device", systemImage: "iphone.slash")
                }
            case .none:
                Button {
                    device.download(episode)
                } label: {
                    Label("Download to device", systemImage: "arrow.down.to.line.circle")
                }
            }
        }
    }

    @ViewBuilder
    private var contextSection: some View {
        switch context {
        case .queue:
            Button(role: .destructive) {
                core.models?.queue.remove(episode)
            } label: {
                Label("Remove from Queue", systemImage: "minus.circle")
            }
        case .playlist(let name, let model):
            Button(role: .destructive) {
                // Same path as the swipe action: model.remove updates the
                // visible list + cache AND enqueues the op.
                model.remove(episode)
            } label: {
                Label("Remove from \(name)", systemImage: "minus.circle")
            }
        case .browse:
            EmptyView()
        }
    }
}

/// An episode list row (web item.rs organization, native widgets). Navigation
/// is DELIBERATELY narrow: only title, artwork, and trailing chevron push the
/// episode detail — a missed tap near the action icons never navigates.
struct EpisodeRowLink: View {
    let episode: EpisodeData
    let artURL: URL?
    var subtitle: String? = nil
    let context: EpisodeMenuContext
    let core: HalogenCore

    @Environment(Navigator.self) private var navigator: Navigator?

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            // Row 1: artwork | title + podcast (stacked) | chevron.
            HStack(alignment: .center, spacing: 12) {
                detailTapTarget {
                    Artwork(url: artURL, size: 48)
                }
                VStack(alignment: .leading, spacing: 2) {
                    detailTapTarget {
                        Text(episode.title)
                            .font(.subheadline.weight(.medium))
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .accessibilityIdentifier("row-title")
                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                detailTapTarget {
                    Image(systemName: "chevron.right")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .frame(width: 24, height: 40)
                }
            }
            // Row 2: one-line description, running to the edge unless the
            // status icon occupies it. Evaluated ONCE per body (it parses HTML).
            let preview = descriptionPreview
            if preview != nil || marker != .none {
                HStack(alignment: .center, spacing: 8) {
                    if let preview {
                        Text(preview)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                    PlaybackMarkerIcon(marker: marker)
                        .font(.caption)
                }
            }
            // Row 3: play, download/trash, duration, published — and the
            // three dots pulled to the far right (under chevron/status).
            HStack(spacing: 14) {
                playPauseButton
                DownloadButton(episode: episode, core: core)
                HStack(spacing: 6) {
                    if let secs = episode.duration_secs, secs > 0 {
                        Text(EpisodeRowStyle.duration(secs))
                    }
                    if let published = episode.published_at {
                        Text("·")
                        Text(published, format: .relative(presentation: .named))
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Menu {
                    EpisodeMenu(episode: episode, context: context, core: core)
                } label: {
                    Image(systemName: "ellipsis")
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
            }
            // Row 4 (optional): live progress — the NOW PLAYING episode only.
            if let progress {
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
                    .tint(Color.accentColor)
            }
        }
        .padding(.vertical, 2)
        .contextMenu {
            EpisodeMenu(episode: episode, context: context, core: core)
        }
        // No container identifier here: SwiftUI propagates it onto every
        // child, clobbering the buttons' own identifiers (row-play etc.).
    }

    /// Make JUST this piece a tap target for the detail push: a plain Button's
    /// hit area is exactly the label's frame (hidden NavigationLinks all
    /// activate together on a List row tap).
    private func detailTapTarget<Label: View>(
        @ViewBuilder _ label: () -> Label
    ) -> some View {
        Button {
            navigator?.push(.episode(episode.id))
        } label: {
            label().contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// One-line plain-text preview of the (HTML) feed description — memoized
    /// in HTMLText (row bodies re-render on every overlay/player publish).
    private var descriptionPreview: String? {
        guard let html = episode.description, !html.isEmpty else { return nil }
        let plain = HTMLText.preview(html)
        return plain.isEmpty ? nil : plain
    }

    /// Play flips to pause while THIS episode is the playing one (tap then
    /// pauses in place); a paused current episode shows play and resumes.
    private var playPauseButton: some View {
        let player = core.models?.player
        let isCurrent = player?.current?.id == episode.id
        let playing = isCurrent && (player?.isPlaying ?? false)
        return Button {
            if isCurrent {
                player?.toggle()
            } else {
                player?.play(episode, context: context.playbackContext)
            }
        } label: {
            Image(systemName: playing ? "pause.circle" : "play.circle")
                .font(.title3)
                .foregroundStyle(Color.accentColor)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityIdentifier("row-play")
    }

    // MARK: - playback state (overlay-wins, like every other reader)

    /// The row's marker — next-up outranks the played facet in the queue
    /// (web item.rs: the PlaybackMarker priority).
    private var marker: EpisodeRowStyle.PlaybackMarker {
        if isNextUp { return .nextUp }
        let status = core.models?.playbacks.status(for: episode)
            ?? episode.playback_status ?? .unplayed
        switch status {
        case .finished: return .finished
        case .played: return .inProgress
        case .unplayed: return .none
        }
    }

    /// The episode that plays when the current one ends — shown only in the
    /// list playback actually continues through (web next_up_in): the active
    /// play-context playlist's own page, else the queue.
    private var isNextUp: Bool {
        let contextId = core.models?.player.contextPlaylistId
        let episodes: [EpisodeData]
        switch context {
        case .queue:
            guard contextId == nil else { return false }
            episodes = core.models?.queue.episodes ?? []
        case .playlist(_, let model):
            guard contextId == model.playlistId else { return false }
            episodes = model.episodes
        case .browse:
            return false
        }
        guard !episodes.isEmpty else { return false }
        let nextId: Int32?
        if let currentId = core.models?.player.current?.id,
            let idx = episodes.firstIndex(where: { $0.id == currentId })
        {
            nextId = idx + 1 < episodes.count ? episodes[idx + 1].id : nil
        } else {
            nextId = episodes.first?.id
        }
        return nextId == episode.id
    }

    /// Thin-track fraction: real progress only (web compute_progress). The
    /// now-playing row tracks the LIVE position; other rows read only the saved
    /// cursor, so they never subscribe to `position`'s ~4×/sec ticks.
    private var progress: Double? {
        guard let duration = episode.duration_secs, duration > 0 else { return nil }
        let player = core.models?.player
        let cursor: Double
        if player?.current?.id == episode.id, let pos = player?.position, pos > 0 {
            cursor = pos
        } else if let saved = core.models?.playbacks.cursor(for: episode)
            ?? episode.playback?.cursor
        {
            cursor = Double(saved)
        } else {
            return nil
        }
        let frac = cursor / Double(duration)
        return frac > 0 && frac < 1 ? frac : nil
    }
}

/// The row's download control, mirroring the web's tiered icons: trash (on
/// device), ring on the download glyph (device downloading), ring on the CLOUD
/// glyph (server downloading), cloud-download (server-only → pull to device),
/// plain download (nowhere → full server-then-device chain).
struct DownloadButton: View {
    let episode: EpisodeData
    let core: HalogenCore

    var body: some View {
        // Embedded accounts never device-download (the media already lives in
        // the on-device server) — the button manages the SERVER copy there.
        let device = core.isEmbeddedAccount
            ? .none : (core.models?.device.state(of: episode.id) ?? .none)
        let serverProgress = core.models?.serverDownloads.progress(of: episode.id)
        let onServer = core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)
        let serverRunning =
            serverProgress != nil || episode.download_status == .downloading
        let serverFailed = core.models?.serverDownloads.failed.contains(episode.id) == true

        Button {
            switch device {
            case .downloaded:
                core.models?.device.remove(episode.id)
            case .downloading, .waitingServer:
                core.models?.device.pause(episode.id)
            case .paused, .failed:
                core.models?.device.download(episode)
            case .none:
                if serverRunning {
                    core.models?.serverDownloads.watch(episode.id)
                } else if onServer {
                    if core.isEmbeddedAccount {
                        // Optimistic overlay first — the trash flips back to
                        // a download glyph immediately.
                        core.models?.serverDownloads.markRemovedLocally(episode.id)
                        Task {
                            await core.outbox?.enqueue(
                                .removeServerDownload(episodeId: episode.id))
                        }
                    } else {
                        core.models?.device.download(episode)
                    }
                } else if core.isEmbeddedAccount {
                    core.models?.serverDownloads.download(episode)
                } else {
                    // Full chain: server fetch (cloud ring), then pull to the
                    // device. Server-only downloads live in the context menu.
                    core.models?.device.download(episode)
                }
            }
        } label: {
            Group {
                switch device {
                case .downloaded:
                    Image(systemName: "trash")
                        .foregroundStyle(.red)
                case .waitingServer:
                    // Server phase of a chained device download — cloud ring
                    // tracking the server's own progress.
                    ring(
                        progress: serverProgress ?? 0, glyph: "icloud.and.arrow.down",
                        tint: Color.accentColor)
                case .downloading(let progress):
                    ring(progress: progress, glyph: "arrow.down", tint: Color.accentColor)
                case .paused:
                    Image(systemName: "pause.circle")
                        .foregroundStyle(.orange)
                case .failed(let message):
                    // Surfaced failure (tap retries) — never disguised as a
                    // pause (web: the Failed badge).
                    Image(systemName: "exclamationmark.circle")
                        .foregroundStyle(.red)
                        .accessibilityLabel("Download failed: \(message) Tap to retry.")
                case .none:
                    if serverRunning {
                        ring(
                            progress: serverProgress ?? 0, glyph: "icloud.and.arrow.down",
                            tint: Color.accentColor)
                    } else if serverFailed, !onServer {
                        // Failed server fetch — distinct from "never
                        // downloaded"; tap retries the chain.
                        Image(systemName: "exclamationmark.icloud")
                            .foregroundStyle(.red)
                            .accessibilityLabel("Server download failed — tap to retry")
                    } else if onServer {
                        if core.isEmbeddedAccount {
                            Image(systemName: "trash")
                                .foregroundStyle(.red)
                        } else {
                            Image(systemName: "icloud.and.arrow.down")
                                .foregroundStyle(Color.accentColor)
                        }
                    } else {
                        Image(systemName: "arrow.down.circle")
                            .foregroundStyle(Color.accentColor)
                    }
                }
            }
            .font(.title3)
            .frame(width: 30, height: 32)
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .onAppear {
            // A row already DOWNLOADING server-side gets live tracking.
            if device == .none, episode.download_status == .downloading {
                core.models?.serverDownloads.watch(episode.id)
            }
        }
    }

    /// Progress ring wrapped around a glyph — the shared spinner treatment.
    private func ring(progress: Double, glyph: String, tint: Color) -> some View {
        ZStack {
            Circle().stroke(Color(.systemGray4), lineWidth: 2)
            Circle()
                .trim(from: 0, to: max(progress, 0.04))
                .stroke(tint, lineWidth: 2)
                .rotationEffect(.degrees(-90))
            Image(systemName: glyph)
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(tint)
        }
        .frame(width: 20, height: 20)
    }
}
