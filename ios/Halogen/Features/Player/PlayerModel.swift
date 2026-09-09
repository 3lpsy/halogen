import AVFoundation
import Foundation
import MediaPlayer
import Observation
import UIKit

/// Playback: AVPlayer streaming from the server's audio endpoint (bearer via
/// asset headers), background audio, lock-screen controls, cursor persistence
/// through the outbox (offline-safe), and continuation on finish through the
/// play-context playlist or the queue (the web's on_ended/next_up_in).
@MainActor
@Observable
final class PlayerModel {
    private let accountStore: LocalStore?

    private unowned let core: HalogenCore

    private(set) var current: EpisodeData?
    private(set) var isPlaying = false
    private(set) var duration: Double = 0
    /// Server-download progress while an un-downloaded episode is prepared.
    private(set) var preparing: Bool = false
    /// What the preparation is waiting on (mini-player caption).
    private(set) var preparingLabel: String = "Preparing episode…"
    /// True while audio comes over the network (the strategy-visible state).
    private(set) var streaming = false
    /// A remote stream's buffer ran dry and AVPlayer is rebuffering — the
    /// transports show a spinner instead of a playing icon over silence.
    private(set) var buffering = false
    /// Sticky failure shown in the mini player (tap play to retry).
    private(set) var failureMessage: String?
    var position: Double = 0
    var rate: Float = 1.0
    /// Sleep timer: counts down ONLY while actually playing (web sleep.rs —
    /// "a paused player holds the timer"); expiry pauses playback once.
    private(set) var sleepRemainingSecs: Double?
    private(set) var sleepAtEpisodeEnd = false
    /// Once-per-session latch for `sleepByDefault` auto-arm (web:
    /// sleep_auto_armed) — a manual disable isn't undone by the next episode.
    private var sleepAutoArmed = false
    /// Set when the minutes timer expires exactly as the episode ends —
    /// suppresses the coincident auto-advance (web: on_ended(!sleep_expired)).
    private var suppressAdvanceOnEnd = false
    /// Wall-clock instant of the last playing tick (the countdown decrements
    /// by real elapsed time, independent of playback rate).
    private var lastSleepTick: Date?

    /// The playlist the user pressed play from (the web's PlayContext):
    /// auto-advance / Up-next / transport Next continue through THIS list;
    /// nil = queue semantics. Set by the `context:` play entries, preserved
    /// by continuation paths, cleared by stop().
    private(set) var contextPlaylistId: Int32?
    /// Membership snapshot of the context playlist (position order), taken at
    /// play time and kept by the owning screen's optimistic model.
    private var contextEpisodes: [EpisodeData] = []

    /// The now-playing episode's chapter markers (lazily fetched when the row
    /// snapshot lacks them — the web's ensure_episode_chapters).
    private(set) var chapters: [EpisodeChapterData] = []

    private var player: AVPlayer?
    private var timeObserver: Any?
    private var endObserver: NSObjectProtocol?
    private var statusObservation: NSKeyValueObservation?
    private var timeControlObservation: NSKeyValueObservation?
    private var ticksSinceSave = 0
    /// Loading-stall watchdog (web: LOADING_STALL_TICKS ≈ 30s) — a stream
    /// that never reaches readyToPlay surfaces an error instead of showing
    /// "playing" forever.
    private var stallTask: Task<Void, Never>?
    /// Post-ready dry-buffer guard (see bufferingChanged).
    private var rebufferTask: Task<Void, Never>?
    /// Resume target applied once the item reaches readyToPlay — seeking an
    /// unready item can wedge preparation.
    private var pendingResume: Double = 0

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
        configureRemoteCommands()
        observeAudioSession()
    }

    /// System audio-session events: a call/Siri/alarm pauses us via an
    /// interruption (persist the cursor at that edge and reflect the paused
    /// state; auto-resume when the system says so), and unplugging
    /// headphones pauses rather than blasting the speaker.
    private func observeAudioSession() {
        NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: AVAudioSession.sharedInstance(), queue: .main
        ) { [weak self] note in
            guard
                let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                let type = AVAudioSession.InterruptionType(rawValue: raw)
            else { return }
            Task { @MainActor in
                guard let self else { return }
                switch type {
                case .began:
                    self.saveCursorNow()
                    self.isPlaying = false
                case .ended:
                    let optRaw =
                        note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
                    let options = AVAudioSession.InterruptionOptions(rawValue: optRaw)
                    if options.contains(.shouldResume), self.current != nil {
                        try? AVAudioSession.sharedInstance().setActive(true)
                        self.player?.rate = self.rate
                        self.isPlaying = true
                    }
                @unknown default:
                    break
                }
            }
        }
        NotificationCenter.default.addObserver(
            forName: AVAudioSession.routeChangeNotification,
            object: AVAudioSession.sharedInstance(), queue: .main
        ) { [weak self] note in
            guard
                let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                AVAudioSession.RouteChangeReason(rawValue: raw) == .oldDeviceUnavailable
            else { return }
            Task { @MainActor in
                guard let self, self.isPlaying else { return }
                self.saveCursorNow()
                self.toggle()
            }
        }
    }

    // MARK: - controls

    /// The continuation playlist a play was launched from (web PlayContext):
    /// its id plus a membership snapshot in position order.
    struct PlaybackContext {
        let playlistId: Int32
        let episodes: [EpisodeData]
    }

    /// Continuation play (auto-advance, transport next/prev, retry): keeps
    /// the current play context — the web's context-less request_play.
    func play(_ episode: EpisodeData) {
        start(episode, forceStream: false)
    }

    /// User-entry play from a list (web request_play_in): sets the play
    /// context first; nil resets to queue semantics.
    func play(_ episode: EpisodeData, context: PlaybackContext?) {
        setContext(context)
        start(episode, forceStream: false)
    }

    /// Explicit server streaming — the web's "Stream from server" escape
    /// hatch (stream_episode_in): bypasses the device copy and the strategy
    /// tree; a missing server copy is prepared there first.
    func stream(_ episode: EpisodeData, context: PlaybackContext? = nil) {
        setContext(context)
        start(episode, forceStream: true)
    }

    private func setContext(_ context: PlaybackContext?) {
        contextPlaylistId = context?.playlistId
        contextEpisodes = context?.episodes ?? []
    }

    /// `forceRestart` rebuilds the pipeline even when `episode` is already
    /// current — nil-ing `current` to fake this unmounts the mini player AND
    /// the presented sheet (iOS dismisses it; the e2e Up Next failure).
    private var resolvedLocalAudio: [Int32: URL] = [:]
    private var localResolution: Task<Void, Never>?

    private func start(_ episode: EpisodeData, forceStream: Bool, forceRestart: Bool = false) {
        // A failed item can't be revived by just setting rate — a retry must
        // rebuild the source (web: play() from Error routes through
        // request_play's full re-resolution).
        let retrying =
            current?.id == episode.id
            && (failureMessage != nil || player?.currentItem?.status == .failed)
        failureMessage = nil
        if !retrying, !forceRestart, current?.id == episode.id, let player {
            player.rate = rate
            isPlaying = true
            return
        }
        saveCursorNow()
        teardown()
        current = episode
        // Zero the playhead IMMEDIATELY: any saveCursorNow() before the resume
        // cursor is computed would persist the PREVIOUS episode's position
        // against this one (web: persist_cursor skips Preparing).
        position = 0
        pendingResume = 0
        preparing = false
        suppressAdvanceOnEnd = false
        chapters = episode.chapters ?? []
        if episode.chapters == nil { loadChapters(for: episode) }
        // Auto-arm the sleep timer once per listening session when the pref
        // is set (web: load_and_play → maybe_auto_arm_sleep).
        if core.models?.prefs.prefs.sleepByDefault == true, !sleepAutoArmed,
            sleepRemainingSecs == nil, !sleepAtEpisodeEnd
        {
            setSleepTimer(minutes: core.models?.prefs.prefs.defaultSleepMinutes ?? 30)
            sleepAutoArmed = true
        }

        // Strategy-driven sourcing (the web's PlaybackPreference; embedded is
        // forced to streamOnly). The audio endpoint only serves files the
        // server has stored, so "prepare" first triggers that download.
        // An explicit stream request pins the strategy to streamOnly.
        let strategy: ClientPrefs.PlaybackStrategy =
            forceStream ? .streamOnly : core.effectivePlaybackStrategy
        let hasDevice = core.models?.device.localURL(episodeId: episode.id) != nil
        let onServer =
            core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)

        switch strategy {
        case .streamOnly:
            if !onServer {
                prepareViaServer(episode)
                return
            }
        case .streamFallback:
            // Local first; stream only as fallback. Never device-downloads.
            if !hasDevice && !onServer {
                prepareViaServer(episode)
                return
            }
        case .streamFirstAndDownload:
            if !onServer {
                prepareViaServer(episode)
                return
            }
            // Stream now, pull the device copy in the background for later.
            if !hasDevice && !core.isEmbeddedAccount {
                core.models?.device.download(episode)
            }
        case .downloadOnly:
            if !hasDevice {
                // Never streams: server copy first (if needed), then the
                // device copy, then play local bytes.
                prepareDeviceCopy(episode, needsServer: !onServer)
                return
            }
        }

        if core.isEmbeddedAccount, resolvedLocalAudio[episode.id] == nil {
            localResolution?.cancel()
            localResolution = Task { [weak self] in
                guard let self else { return }
                do {
                    guard let url = try await core.forAccount(accountStore).localAudioURL(episodeId: episode.id) else {
                        failureMessage = "Episode audio is not downloaded"
                        isPlaying = false
                        return
                    }
                    guard !Task.isCancelled, current?.id == episode.id else { return }
                    resolvedLocalAudio[episode.id] = url
                    start(episode, forceStream: forceStream, forceRestart: true)
                } catch {
                    guard !Task.isCancelled else { return }
                    failureMessage = FriendlyError.message(error)
                    isPlaying = false
                }
            }
            return
        }

        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
        try? AVAudioSession.sharedInstance().setActive(true)

        // Prefer the on-device copy (works fully offline) unless the strategy
        // is stream-only; stream otherwise.
        let local =
            resolvedLocalAudio[episode.id]
            ?? (strategy == .streamOnly
                ? nil : core.models?.device.localURL(episodeId: episode.id))
        guard var url = local ?? core.audioURL(episodeId: episode.id) else {
            // No device copy and no usable stream URL (manual offline or
            // signed out) — a silent return here mounted a dead mini player.
            isPlaying = false
            failureMessage =
                core.isOffline
                ? "You're offline — download the episode or reconnect to stream it."
                : "Not signed in — can't stream this episode."
            DeviceLog.warn("player: no audio URL for episode \(episode.id)")
            return
        }
        streaming = (local == nil)
        var options: [String: Any] = [:]
        // STREAMS only: the audio URL has no extension, so hint the type so
        // content sniffing can't stall item preparation (iOS 17+). Local
        // copies carry a real extension — never override for them.
        if local == nil {
            options["AVURLAssetOverrideMIMETypeKey"] = Self.streamMIMEHint(for: episode)
        }
        rate = core.models?.prefs.prefs.defaultRate ?? rate
        #if DEBUG
            // Isolation hook: HALOGEN_AUDIO_OVERRIDE plays an arbitrary URL
            // with no auth options (environment-vs-code triage).
            if let raw = ProcessInfo.processInfo.environment["HALOGEN_AUDIO_OVERRIDE"],
                let override = URL(string: raw)
            {
                url = override
            }
        #endif
        if local == nil, let token = core.apiToken {
            options["AVURLAssetHTTPHeaderFieldsKey"] = ["Authorization": "Bearer \(token)"]
        }
        #if DEBUG
            print("player: starting episode \(episode.id) url \(url)")
        #endif
        let asset = AVURLAsset(url: url, options: options)
        #if DEBUG
            Task {
                do {
                    let (playable, duration) = try await asset.load(.isPlayable, .duration)
                    print("player: asset loaded playable=\(playable) duration=\(duration.seconds)")
                } catch {
                    print("player: asset load FAILED: \(error)")
                }
            }
        #endif
        let item = AVPlayerItem(asset: asset)
        let player = AVPlayer(playerItem: item)
        self.player = player
        statusObservation = item.observe(\.status) { [weak self] item, _ in
            #if DEBUG
                print(
                    "player: item status \(item.status.rawValue) error \(String(describing: item.error))"
                )
            #endif
            Task { @MainActor in self?.itemBecameReady(item) }
        }
        // Post-ready stall visibility: when a stream's buffer runs dry the
        // periodic ticks STOP, so this observation is the only signal that
        // playback stalled — surface it as `buffering` instead of a playing
        // icon over silence (web: Buffering reflected into now_playing).
        timeControlObservation = player.observe(\.timeControlStatus) { p, _ in
            #if DEBUG
                print(
                    "player: timeControl \(p.timeControlStatus.rawValue) waiting \(p.reasonForWaitingToPlay?.rawValue ?? "-")"
                )
            #endif
            let waiting = p.timeControlStatus == .waitingToPlayAtSpecifiedRate
            Task { @MainActor [weak self] in
                self?.buffering = waiting
                self?.bufferingChanged(waiting)
            }
        }

        // Resume from the freshest known cursor (seconds) — the local overlay
        // outranks the row snapshot when newer (offline listening progress
        // must not regress). Applied on readyToPlay.
        pendingResume = Double(
            core.models?.playbacks.cursor(for: episode) ?? episode.playback?.cursor ?? 0)
        position = pendingResume
        // Queue/playlist pages can't embed Playback (the server include isn't
        // supported on that route): when neither the overlay nor the row
        // snapshot knows a cursor, ask the episode detail (which does) and
        // late-apply the resume while playback is still at the head.
        if episode.playback == nil, core.models?.playbacks.entries[episode.id] == nil {
            Task { [weak self] in
                guard let self,
                    let fresh = try? await self.core.forAccount(self.accountStore).episodeDetail(id: episode.id),
                    let cursor = fresh.playback?.cursor, cursor > 0
                else { return }
                guard self.current?.id == episode.id, self.position < 5 else { return }
                if self.duration > 0 {
                    self.seek(to: Double(cursor))
                } else {
                    self.pendingResume = Double(cursor)
                    self.position = self.pendingResume
                }
            }
        }

        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 1, preferredTimescale: 2), queue: .main
        ) { [weak self] time in
            Task { @MainActor in self?.tick(time) }
        }
        endObserver = NotificationCenter.default.addObserver(
            forName: AVPlayerItem.didPlayToEndTimeNotification, object: item, queue: .main
        ) { [weak self] note in
            Task { @MainActor in
                // A stale end can land here: a notification queued before
                // teardown() still arrives and would run finished() against
                // the NEW episode (the e2e Up Next race, blank player sheet).
                guard let self, let ended = note.object as? AVPlayerItem,
                    ended === self.player?.currentItem
                else { return }
                self.finished()
            }
        }

        // Loopback sources need no stall management (the simulator's buffering
        // evaluation can wedge a loopback stream forever); a REMOTE stream
        // needs AVPlayer's buffer-wait/recovery for cellular dropouts.
        player.automaticallyWaitsToMinimizeStalling = streaming && !core.isEmbeddedAccount
        player.playImmediately(atRate: rate)
        isPlaying = true
        armLoadingStallGuard(item: item, episodeId: episode.id)
        updateNowPlaying()
        loadArtworkIntoNowPlaying(episode)
    }

    /// Web parity: a stream that never reaches readyToPlay must not show
    /// "playing" forever (controller.rs LOADING_STALL_TICKS ≈ 30s) — surface
    /// a tappable failure instead. The item/episode identity checks make a
    /// superseded guard a no-op.
    private func armLoadingStallGuard(item: AVPlayerItem, episodeId: Int32) {
        stallTask?.cancel()
        stallTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard !Task.isCancelled, let self else { return }
            guard self.current?.id == episodeId,
                self.player?.currentItem === item,
                item.status != .readyToPlay
            else { return }
            self.player?.pause()
            self.isPlaying = false
            self.failureMessage = "Playback timed out — couldn't start this episode."
            DeviceLog.error("player: loading stalled >30s for episode \(episodeId)")
        }
    }

    /// armLoadingStallGuard's shape for AFTER readyToPlay: a mid-episode
    /// network drop leaves the buffer dry with `buffering` true forever and
    /// no failure transition — bound it so the spinner becomes a tappable
    /// failure instead of indefinite silence.
    private func bufferingChanged(_ waiting: Bool) {
        rebufferTask?.cancel()
        rebufferTask = nil
        guard waiting, streaming else { return }
        let episodeId = current?.id
        rebufferTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(45))
            guard !Task.isCancelled, let self else { return }
            guard self.current?.id == episodeId, self.buffering else { return }
            self.player?.pause()
            self.isPlaying = false
            self.buffering = false
            self.failureMessage =
                "Stream stalled — check your connection, then tap play to retry."
            DeviceLog.error(
                "player: rebuffering stalled >45s for episode \(episodeId.map(String.init) ?? "?")"
            )
        }
    }

    /// Actionable copy for an AVPlayerItem failure — network drop, expired
    /// token, and broken file need different next steps; the raw CoreMedia
    /// string distinguishes none of them. Walks the NSUnderlyingError chain.
    private static func streamErrorFacts(_ error: Error?) -> (network: Bool, auth: Bool) {
        var sawNetwork = false
        var sawAuth = false
        var cursor = error as NSError?
        while let e = cursor {
            if e.domain == NSURLErrorDomain {
                sawNetwork = true
                if e.code == NSURLErrorUserAuthenticationRequired
                    || e.code == NSURLErrorNoPermissionsToReadFile
                {
                    sawAuth = true
                }
            }
            cursor = e.userInfo[NSUnderlyingErrorKey] as? NSError
        }
        return (sawNetwork, sawAuth)
    }

    /// The audio stream bypasses the client's 401 machinery (the bearer
    /// header is baked into the asset at build time) — on an auth-shaped
    /// stream failure, probe an authenticated endpoint so the shared token
    /// box refreshes; the retry then rebuilds the asset with a fresh token.
    private func healAuthForStreamFailure(_ error: Error?) {
        guard streaming, let episode = current,
            Self.streamErrorFacts(error).auth
        else { return }
        Task { _ = try? await core.forAccount(accountStore).episodeDetail(id: episode.id) }
    }

    private func playbackFailureMessage(_ error: Error?) -> String {
        let (sawNetwork, sawAuth) = Self.streamErrorFacts(error)
        if streaming {
            if sawAuth {
                return "The stream was refused — tap play to retry, or sign in again."
            }
            if sawNetwork || core.isOffline {
                return "Stream interrupted — check your connection, then tap play to retry."
            }
            return "Couldn't play this stream — tap play to retry, or download the episode."
        }
        return "Couldn't play the downloaded file — it may be damaged. Remove local data and re-download it."
    }

    /// Lazy chapter fetch when the row snapshot has none embedded: cached
    /// detail first (offline), then the network detail (which includes
    /// Chapters) — the web's ensure_episode_chapters.
    private func loadChapters(for episode: EpisodeData) {
        Task { [weak self] in
            guard let self else { return }
            var detail = await self.accountStore?.load(
                EpisodeData.self, key: CacheKey.episode(episode.id))
            if detail?.chapters == nil {
                detail = (try? await self.core.forAccount(self.accountStore).episodeDetail(id: episode.id)) ?? detail
            }
            guard self.current?.id == episode.id, let found = detail?.chapters else { return }
            self.chapters = found
        }
    }

    /// The chapter currently playing — the last marker at or before the
    /// position (web now_playing.rs active_chapter_index).
    var activeChapterIndex: Int? {
        chapters.lastIndex(where: { Double($0.starts_at_secs) <= position })
    }

    func toggle() {
        // From Error a bare rate change can't revive the failed item — route
        // through the full source rebuild (web parity), else the sheet/lock-
        // screen play buttons wedge a dead player showing "playing".
        if let current, failureMessage != nil || player?.currentItem?.status == .failed {
            start(current, forceStream: false)
            return
        }
        guard let player else { return }
        if isPlaying {
            player.pause()
            isPlaying = false
            saveCursorNow()
        } else {
            player.rate = rate
            isPlaying = true
            // Resuming after a sleep-timer pause starts a fresh listening
            // stretch — the expired timer must not eat the next episode end.
            suppressAdvanceOnEnd = false
            lastSleepTick = nil
        }
        updateNowPlaying()
    }

    func seek(to seconds: Double) {
        // A seek supersedes any queued resume cursor — itemBecameReady must
        // not snap a pre-ready chapter-jump/skip back to the saved position
        // (web: the resume offset is handed once to load(); later seeks go
        // straight through with nothing stored to re-apply).
        pendingResume = 0
        position = seconds
        player?.seek(to: CMTime(seconds: seconds, preferredTimescale: 1))
        saveCursorNow()
        updateNowPlaying()
    }

    func skip(_ delta: Double) {
        seek(to: max(0, position + delta))
    }

    /// Prefs-configured jump distances (Settings → Playback).
    var skipForwardSecs: Double {
        Double(core.models?.prefs.prefs.skipForwardSecs ?? 30)
    }

    var skipBackSecs: Double {
        Double(core.models?.prefs.prefs.skipBackSecs ?? 15)
    }

    func setRate(_ new: Float) {
        rate = new
        if isPlaying { player?.rate = new }
        // Persist as the new default so the choice sticks across tracks —
        // play() applies `defaultRate` on every load (web: the now-playing
        // speed picker writes playback_rate to config).
        if core.models?.prefs.prefs.defaultRate != new {
            core.models?.prefs.update { $0.defaultRate = new }
        }
        updateNowPlaying()
    }

    /// nil cancels; 0 = end of episode; otherwise a countdown of `minutes`
    /// of PLAYING time (web sleep.rs: only ticks down while playing).
    func setSleepTimer(minutes: Int?) {
        sleepRemainingSecs = nil
        sleepAtEpisodeEnd = false
        lastSleepTick = nil
        guard let minutes else { return }
        if minutes == 0 {
            sleepAtEpisodeEnd = true
            return
        }
        sleepRemainingSecs = Double(minutes * 60)
    }

    /// Whole minutes remaining, rounded up (web SleepState::remaining_minutes
    /// — the badge reads "1" until the timer truly reaches zero).
    var sleepRemainingMinutes: Int? {
        sleepRemainingSecs.map { Int((($0) / 60).rounded(.up)) }
    }

    /// Count the timer down by real elapsed wall time — called from tick(),
    /// which only fires while playback advances. Pauses once on the expiry
    /// edge and remembers it so a coincident episode end doesn't auto-advance
    /// (web: tick's sleep_expired suppression).
    private func advanceSleepTimer() {
        guard isPlaying, let remaining = sleepRemainingSecs else {
            lastSleepTick = nil
            return
        }
        let now = Date()
        // Ticks are ~1s of playback apart; clamp the wall delta so a
        // backgrounded/suspended stretch can't burn the whole timer at once.
        let dt = lastSleepTick.map { min(max(now.timeIntervalSince($0), 0), 5) } ?? 1
        lastSleepTick = now
        let next = remaining - dt
        if next <= 0 {
            sleepRemainingSecs = nil
            lastSleepTick = nil
            suppressAdvanceOnEnd = true
            toggle()  // pause + cursor save
        } else {
            sleepRemainingSecs = next
        }
    }

    func stop() {
        localResolution?.cancel()
        resolvedLocalAudio.removeAll()
        saveCursorNow()
        teardown()
        current = nil
        chapters = []
        isPlaying = false
        preparing = false
        streaming = false
        // Closing the player ends the listening session: clear the play
        // context and sleep timer, re-arm the auto-arm latch (web: stop()).
        contextPlaylistId = nil
        contextEpisodes = []
        setSleepTimer(minutes: nil)
        sleepAutoArmed = false
        suppressAdvanceOnEnd = false
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    /// downloadOnly: chain server copy (when missing) → device copy → play
    /// the local bytes. Progress captions track each stage.
    private func prepareDeviceCopy(_ episode: EpisodeData, needsServer: Bool) {
        preparing = true
        preparingLabel = needsServer ? "Preparing on server…" : "Downloading to device…"
        if needsServer {
            core.models?.serverDownloads.download(episode)
        } else {
            core.models?.device.download(episode)
        }
        Task { [weak self] in
            var serverDone = !needsServer
            for _ in 0..<900 {
                try? await Task.sleep(for: .seconds(1))
                guard let self, self.current?.id == episode.id else { return }
                if !serverDone {
                    if self.core.models?.serverDownloads.failed.contains(episode.id) == true {
                        self.preparing = false
                        self.failureMessage = "Server download failed"
                        return
                    }
                    if self.core.models?.serverDownloads.completed.contains(episode.id) == true {
                        serverDone = true
                        self.preparingLabel = "Downloading to device…"
                        self.core.models?.device.download(Self.markedDownloaded(episode))
                    }
                    continue
                }
                switch self.core.models?.device.state(of: episode.id) ?? .none {
                case .downloaded:
                    self.preparing = false
                    // forceRestart, NOT current=nil+play: a nil blip
                    // unmounts the mini player and kills the open sheet.
                    self.start(
                        Self.markedDownloaded(episode), forceStream: false,
                        forceRestart: true)
                    return
                case .paused:
                    self.preparing = false
                    self.failureMessage = "Device download paused (tap to resume)"
                    return
                case .failed(let message):
                    self.preparing = false
                    self.failureMessage = message
                    return
                case .none:
                    self.core.models?.device.download(Self.markedDownloaded(episode))
                case .downloading, .waitingServer:
                    break
                }
            }
            self?.preparing = false
            self?.failureMessage = "Timed out preparing the episode"
        }
    }

    /// Un-downloaded episode: kick the server download, mirror its progress
    /// in the mini player, then start streaming when it completes.
    private func prepareViaServer(_ episode: EpisodeData) {
        preparing = true
        preparingLabel = "Preparing on server…"
        core.models?.serverDownloads.download(episode)
        Task { [weak self] in
            for _ in 0..<450 {
                try? await Task.sleep(for: .seconds(1))
                guard let self, self.current?.id == episode.id else { return }
                if self.core.models?.serverDownloads.failed.contains(episode.id) == true {
                    self.preparing = false
                    self.failureMessage = "Server download failed"
                    return
                }
                if self.core.models?.serverDownloads.completed.contains(episode.id) == true {
                    self.preparing = false
                    // forceRestart, NOT current=nil+play (see start()).
                    self.start(
                        Self.markedDownloaded(episode), forceStream: false,
                        forceRestart: true)
                    return
                }
            }
            self?.preparing = false
            self?.failureMessage = "Timed out preparing the episode"
        }
    }

    /// Copy with download_status flipped so the decision tree streams.
    private static func markedDownloaded(_ e: EpisodeData) -> EpisodeData {
        EpisodeData(
            id: e.id, podcast_id: e.podcast_id, title: e.title,
            description: e.description, content_url: e.content_url, guid: e.guid,
            art_url: e.art_url, published_at: e.published_at,
            downloaded_at: e.downloaded_at, content_file_path: e.content_file_path,
            download_size: e.download_size, art_file_path: e.art_file_path,
            download_status: .downloaded, download_started_at: e.download_started_at,
            download_attempts: e.download_attempts, playback_status: e.playback_status,
            duration_secs: e.duration_secs, created_at: e.created_at,
            updated_at: e.updated_at, podcast: e.podcast, playback: e.playback,
            chapters: e.chapters
        )
    }

    /// MIME hint for a server stream: the server copy's stored extension, then
    /// the enclosure URL, then audio/mpeg — a blanket audio/mpeg would break
    /// m4a/ogg episodes (web: playback resolves strictly by type).
    private static func streamMIMEHint(for episode: EpisodeData) -> String {
        let ext =
            episode.content_file_path.map { URL(fileURLWithPath: $0).pathExtension }
            ?? URL(string: episode.content_url)?.pathExtension
            ?? ""
        switch ext.lowercased() {
        case "m4a", "mp4": return "audio/mp4"
        case "aac": return "audio/aac"
        case "ogg", "oga": return "audio/ogg"
        case "opus": return "audio/opus"
        case "wav": return "audio/wav"
        case "flac": return "audio/flac"
        default: return "audio/mpeg"
        }
    }

    // MARK: - internals

    private func itemBecameReady(_ item: AVPlayerItem) {
        // Failure must surface even when ticks never start (a stream that
        // dies before its first frame) — tick()'s truth-sync can't run then.
        if item.status == .failed {
            isPlaying = false
            failureMessage = playbackFailureMessage(item.error)
            healAuthForStreamFailure(item.error)
            DeviceLog.error(
                "player: item failed before ready: \(item.error.map(String.init(describing:)) ?? "?")"
            )
            return
        }
        guard item.status == .readyToPlay else { return }
        if item.duration.isNumeric { duration = item.duration.seconds }
        if pendingResume > 0 {
            player?.seek(to: CMTime(seconds: pendingResume, preferredTimescale: 1))
            pendingResume = 0
        }
    }

    private func tick(_ time: CMTime) {
        position = time.seconds
        if let item = player?.currentItem {
            if item.duration.isNumeric { duration = item.duration.seconds }
            // Truth-sync: a stalled/failed item must not show a pause icon.
            if item.status == .failed {
                isPlaying = false
                failureMessage = playbackFailureMessage(item.error)
                healAuthForStreamFailure(item.error)
                DeviceLog.error(
                    "player: item failed: \(item.error.map(String.init(describing:)) ?? "?")")
            } else if let player {
                isPlaying = player.rate > 0
            }
        }
        // Sleep countdown rides the playback ticks — a paused player holds
        // the timer (web: persist_while_playing → advance_sleep).
        advanceSleepTimer()
        ticksSinceSave += 1
        // Persist roughly every 10s of playback (coalesced in the outbox).
        if ticksSinceSave >= 10 {
            ticksSinceSave = 0
            #if DEBUG
                print("player: tick save at \(Int(position))s rate \(player?.rate ?? -1)")
            #endif
            saveCursorNow()
        }
        updateNowPlayingElapsed()
    }

    /// End of episode: mark finished, then auto-advance to the item AFTER the
    /// current one in the continuation playlist (the play context, else the
    /// queue) when the setting allows — the web's on_ended. The queue is
    /// never mutated on completion (the web doesn't remove finished items).
    private func finished() {
        guard let episode = current else { return }
        // Optimistic overlay + outbox: lists/menus/History flip immediately.
        core.models?.playbacks.markPlayed(episode, played: true)
        // Zero `position` BEFORE stop()/play() so saveCursorNow() can't
        // re-persist the ~duration end position (web: persist_cursor skips
        // `Ended`). `current` deliberately stays set — nil-ing it blipped the
        // mini player away and dismissed an open sheet during auto-advance.
        position = 0
        if sleepAtEpisodeEnd || suppressAdvanceOnEnd {
            // The user asked to stop here — that wins over the continuation
            // (web: on_ended(!sleep_expired)).
            suppressAdvanceOnEnd = false
            setSleepTimer(minutes: nil)
            stop()
            return
        }
        let autoAdvance = core.models?.prefs.prefs.autoAdvance ?? true
        if autoAdvance, let next = nextUp(after: episode) {
            play(next)
        } else {
            stop()
        }
    }

    // MARK: - continuation (play context → queue), web navigation.rs

    /// The list playback continues through: the play-context playlist when set,
    /// else the queue (web next_up_in). Membership resolves LIVE from the pool's
    /// `episode_ids` — removed episodes must not play next, reorders are honored;
    /// the play-time snapshot + queue are the object cache (unknown rows skipped).
    private func continuationList() -> [EpisodeData] {
        guard let contextPlaylistId else { return core.models?.queue.episodes ?? [] }
        guard
            let ids = core.models?.playlists.playlists
                .first(where: { $0.id == contextPlaylistId })?.episode_ids
        else { return contextEpisodes }
        var byId: [Int32: EpisodeData] = [:]
        for episode in core.models?.queue.episodes ?? [] { byId[episode.id] = episode }
        for episode in contextEpisodes { byId[episode.id] = episode }
        return ids.compactMap { byId[$0] }
    }

    /// The episode that should play next after `episode` (web
    /// PlaylistState::next_up_in): its position-neighbor when it's in the
    /// continuation list (nil past the end — playback stops at the tail),
    /// else the list's head.
    private func nextUp(after episode: EpisodeData) -> EpisodeData? {
        let list = continuationList()
        if let idx = list.firstIndex(where: { $0.id == episode.id }) {
            return idx + 1 < list.count ? list[idx + 1] : nil
        }
        return list.first
    }

    /// The episode `delta` (±1) places from the current one in the active
    /// list: the context playlist when the current episode is in it, else the
    /// queue (web navigation::adjacent_episode; the podcast-order fallback is
    /// omitted — iOS keeps no full podcast episode list in memory).
    private func adjacentEpisode(_ delta: Int) -> EpisodeData? {
        guard let current else { return nil }
        var lists: [[EpisodeData]] = []
        if contextPlaylistId != nil { lists.append(continuationList()) }
        lists.append(core.models?.queue.episodes ?? [])
        for list in lists where list.contains(where: { $0.id == current.id }) {
            guard let idx = list.firstIndex(where: { $0.id == current.id }) else { continue }
            let target = idx + delta
            return target >= 0 && target < list.count ? list[target] : nil
        }
        return nil
    }

    /// Transport Next target: the continuation neighbor, else nil (web adds a
    /// podcast-order fallback; see `adjacentEpisode`).
    private func nextEpisodeTarget() -> EpisodeData? {
        guard let current else { return nil }
        return nextUp(after: current)
    }

    /// The "Up next" preview target (web UpNext over next_up_in).
    var upNext: EpisodeData? { nextEpisodeTarget() }

    var hasNext: Bool { nextEpisodeTarget() != nil }
    var hasPrevious: Bool { adjacentEpisode(-1) != nil }

    /// UI/lock-screen transport: jump to the next episode (continuation
    /// playlist → queue). Preserves the play context (web play_next_episode).
    func playNextEpisode() {
        guard let next = nextEpisodeTarget() else { return }
        play(next)
    }

    /// UI/lock-screen transport: jump to the previous episode.
    func playPreviousEpisode() {
        guard let prev = adjacentEpisode(-1) else { return }
        play(prev)
    }

    private func saveCursorNow() {
        // Never persist while preparing: nothing has played yet, and the
        // position field is not this episode's playhead (web persist_cursor
        // refuses in Preparing for the same reason).
        guard let episode = current, position > 0, !preparing else { return }
        let cursor = UInt64(max(0, position))
        // Overlay first (so every list/detail/relaunch resumes here), then
        // the coalescing outbox op — the web's SetCursor command in order.
        core.models?.playbacks.setCursor(episode, cursor: cursor)
    }

    private func teardown() {
        stallTask?.cancel()
        stallTask = nil
        rebufferTask?.cancel()
        rebufferTask = nil
        if let timeObserver, let player {
            player.removeTimeObserver(timeObserver)
        }
        timeObserver = nil
        if let endObserver {
            NotificationCenter.default.removeObserver(endObserver)
        }
        endObserver = nil
        statusObservation = nil
        timeControlObservation = nil
        player?.pause()
        player = nil
        duration = 0
    }

    // MARK: - media session (lock screen / control center)

    private func configureRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        center.playCommand.addTarget { [weak self] _ in
            Task { @MainActor in if self?.isPlaying == false { self?.toggle() } }
            return .success
        }
        center.pauseCommand.addTarget { [weak self] _ in
            Task { @MainActor in if self?.isPlaying == true { self?.toggle() } }
            return .success
        }
        center.togglePlayPauseCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.toggle() }
            return .success
        }
        // Configured intervals, same as the in-app buttons (web: 'Configured
        // skip intervals (also used by the hardware media controls)').
        // preferredIntervals refreshes in updateNowPlaying so a Settings
        // change reaches the lock screen without a restart.
        center.skipForwardCommand.preferredIntervals = [
            NSNumber(value: skipForwardSecs)
        ]
        center.skipForwardCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.skip(self?.skipForwardSecs ?? 30) }
            return .success
        }
        center.skipBackwardCommand.preferredIntervals = [
            NSNumber(value: skipBackSecs)
        ]
        center.skipBackwardCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.skip(-(self?.skipBackSecs ?? 15)) }
            return .success
        }
        center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else {
                return .commandFailed
            }
            Task { @MainActor in self?.seek(to: event.positionTime) }
            return .success
        }
        // Next/previous episode (web media-session actions), enabled
        // dynamically in updateNowPlaying so the lock screen greys list ends.
        // With mediaNextPrevSeek set they SEEK instead — Bluetooth devices
        // without seek buttons send track commands (web parity).
        center.nextTrackCommand.isEnabled = false
        center.nextTrackCommand.addTarget { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if self.mediaNextPrevSeek {
                    self.skip(self.skipForwardSecs)
                } else {
                    self.playNextEpisode()
                }
            }
            return .success
        }
        center.previousTrackCommand.isEnabled = false
        center.previousTrackCommand.addTarget { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if self.mediaNextPrevSeek {
                    self.skip(-self.skipBackSecs)
                } else {
                    self.playPreviousEpisode()
                }
            }
            return .success
        }
    }

    /// Bluetooth next/prev-as-seek override (web media_next_prev_seek).
    private var mediaNextPrevSeek: Bool {
        core.models?.prefs.prefs.mediaNextPrevSeek ?? false
    }

    private func updateNowPlaying() {
        let center = MPRemoteCommandCenter.shared()
        // Seek-override mode: track commands work whenever something is
        // loaded; otherwise they follow the continuation list's ends.
        let seekMode = mediaNextPrevSeek
        center.nextTrackCommand.isEnabled = seekMode ? current != nil : hasNext
        center.previousTrackCommand.isEnabled = seekMode ? current != nil : hasPrevious
        center.skipForwardCommand.preferredIntervals = [NSNumber(value: skipForwardSecs)]
        center.skipBackwardCommand.preferredIntervals = [NSNumber(value: skipBackSecs)]
        guard let episode = current else { return }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: episode.title,
            MPMediaItemPropertyArtist: episode.podcast?.title ?? "Halogen",
            MPNowPlayingInfoPropertyElapsedPlaybackTime: position,
            MPNowPlayingInfoPropertyPlaybackRate: isPlaying ? Double(rate) : 0,
        ]
        if duration > 0 {
            info[MPMediaItemPropertyPlaybackDuration] = duration
        }
        // Keep existing artwork if already set.
        if let art = MPNowPlayingInfoCenter.default().nowPlayingInfo?[MPMediaItemPropertyArtwork] {
            info[MPMediaItemPropertyArtwork] = art
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    private func updateNowPlayingElapsed() {
        guard var info = MPNowPlayingInfoCenter.default().nowPlayingInfo else { return }
        info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = position
        if duration > 0 { info[MPMediaItemPropertyPlaybackDuration] = duration }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    private func loadArtworkIntoNowPlaying(_ episode: EpisodeData) {
        guard let url = core.episodeArtURL(episode, small: false) else { return }
        Task {
            guard let image = await ArtLoader.shared.image(for: url) else { return }
            var info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [:]
            info[MPMediaItemPropertyArtwork] = MPMediaItemArtwork(boundsSize: image.size) { _ in
                image
            }
            MPNowPlayingInfoCenter.default().nowPlayingInfo = info
        }
    }
}
