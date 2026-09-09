import SwiftUI

/// Root: resumes the stored session (or lands on auth), then hands over to
/// the tab dock.
struct RootView: View {
    @State private var core = HalogenCore()
    @State private var showRecoveryPurge = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            switch core.phase {
            case .ready:
                if let models = core.models {
                    // Keyed by account: an account switch remounts the tabs
                    // (and their models) over the new namespace.
                    HomeTabs(core: core, models: models)
                        .id(core.account?.namespace)
                        .safeAreaInset(edge: .top, spacing: 0) {
                            if let failure = core.storageFailure {
                                StorageFailureBanner(message: failure)
                            }
                        }
                }
            case .needsAuth:
                ConnectView(core: core)
            case .failed(let message):
                // Never a dead end: retry the boot, or drop the stored
                // session and land on the connect page.
                ContentUnavailableView {
                    Label("Couldn't start", systemImage: "xmark.octagon")
                } description: {
                    Text(message).font(.footnote.monospaced())
                } actions: {
                    Button("Try again") {
                        Task { await core.retryBoot() }
                    }
                    .buttonStyle(.borderedProminent)
                    // The web's standalone /cache-control page renders even
                    // mid-failure: a wedged store or a corrupt embedded
                    // library must always be purgeable in-app.
                    Button("Local data…") {
                        showRecoveryPurge = true
                    }
                    Button("Sign out", role: .destructive) {
                        core.signOut()
                    }
                }
                .sheet(isPresented: $showRecoveryPurge) {
                    NavigationStack {
                        LocalDataView(core: core)
                    }
                }
            case .idle, .starting:
                BootSplash()
            }
        }
        .preferredColorScheme(
            (core.models?.prefs.prefs.theme ?? .dark).colorScheme
        )
        .dynamicTypeSize((core.models?.prefs.prefs.uiSize ?? .medium).dynamicType)
        .toastOverlay()
        .task { await core.boot() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                Task { await core.foregroundSync() }
            }
        }
    }
}

/// The tab dock: the first N visible nav destinations (user-configurable
/// order/visibility — Settings → Configure dock) plus the always-present
/// More tab, mirroring the web's dock + `/menu` model.
private struct HomeTabs: View {
    let core: HalogenCore
    let models: Models

    /// DEBUG smoke-test hook: HALOGEN_TAB selects the initial tab headlessly
    /// (a BuiltinNav rawValue, or "more").
    @State private var selection: String = {
        #if DEBUG
            return ProcessInfo.processInfo.environment["HALOGEN_TAB"] ?? BuiltinNav.queue.rawValue
        #else
            return BuiltinNav.queue.rawValue
        #endif
    }()

    var body: some View {
        TabView(selection: $selection) {
            ForEach(models.nav.dockItems(isAdmin: core.isAdmin)) { item in
                TabStack(core: core, models: models) {
                    BuiltinDestination(item: item, core: core, models: models)
                }
                .tabItem { Label(item.label, systemImage: item.systemImage) }
                .tag(item.rawValue)
            }

            TabStack(core: core, models: models) {
                MoreView(core: core, models: models)
            }
            .tabItem { Label("More", systemImage: "ellipsis") }
            .tag("more")
        }
        // Global add-to-playlist picker (the web's picker page): swipe/bulk
        // "Add to playlist" queues episodes on PlaylistsModel.pendingPick and
        // this dialog resolves the choice wherever the user is.
        .confirmationDialog(
            playlistPickTitle,
            isPresented: Binding(
                get: { !models.playlists.pendingPick.isEmpty },
                set: { if !$0 { models.playlists.pendingPick = [] } }
            ),
            titleVisibility: .visible
        ) {
            ForEach(models.playlists.playlists.filter { !$0.is_default }, id: \.id) { playlist in
                Button(playlist.name) {
                    for episode in models.playlists.pendingPick {
                        models.playlists.add(episode, to: playlist)
                    }
                    models.playlists.pendingPick = []
                }
            }
            Button("Cancel", role: .cancel) { models.playlists.pendingPick = [] }
        }
        .task { await debugAutoplay() }
        .task { await debugAutodownload() }
    }

    private var playlistPickTitle: String {
        let pending = models.playlists.pendingPick
        if pending.count == 1, let first = pending.first {
            return "Add “\(first.title)” to playlist"
        }
        return "Add \(pending.count) episodes to playlist"
    }

    /// DEBUG smoke-test hook: HALOGEN_AUTOPLAY=<episode-id> starts playback
    /// headlessly (exercises AVPlayer streaming + the mini player).
    private func debugAutoplay() async {
        #if DEBUG
            guard let raw = ProcessInfo.processInfo.environment["HALOGEN_AUTOPLAY"],
                let id = Int32(raw)
            else { return }
            if let episode = try? await core.episodeDetail(id: id) {
                models.player.play(episode)
            }
        #endif
    }

    /// DEBUG smoke-test hook: HALOGEN_AUTODOWNLOAD=<episode-id> starts a
    /// device download headlessly (exercises the chunk/resume loop).
    private func debugAutodownload() async {
        #if DEBUG
            guard let raw = ProcessInfo.processInfo.environment["HALOGEN_AUTODOWNLOAD"],
                let id = Int32(raw)
            else { return }
            if let episode = try? await core.episodeDetail(id: id) {
                models.device.download(episode)
            }
        #endif
    }
}

/// One tab's NavigationStack: owns the tab's Navigator (programmatic path)
/// and injects it so rows/menus can push AppRoutes without NavigationLinks.
private struct TabStack<Content: View>: View {
    let core: HalogenCore
    let models: Models
    @ViewBuilder let content: () -> Content

    @State private var navigator = Navigator()

    var body: some View {
        NavigationStack(path: $navigator.path) {
            content()
                .appDestinations(core: core, models: models)
        }
        .environment(navigator)
        // On the STACK, not its root view: pushed screens replace the root,
        // so an inset attached inside vanished on every navigation (no mini
        // player on podcast/episode detail).
        .safeAreaInset(edge: .bottom, spacing: 0) {
            MiniPlayerBar(player: models.player, core: core)
        }
    }
}

/// Persistent warning when the account's LocalStore failed to open: without
/// it every optimistic mutation no-ops silently (nil outbox) while the app
/// looks fully functional.
private struct StorageFailureBanner: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "externaldrive.badge.exclamationmark")
            .font(.caption)
            .multilineTextAlignment(.leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background(.red.opacity(0.88))
            .foregroundStyle(.white)
    }
}

/// Shown while a stored session resumes (sub-second warm; a few seconds on
/// the embedded server's first provision).
private struct BootSplash: View {
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "waveform.circle.fill")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
            Text("Halogen").font(.largeTitle.bold())
            ProgressView()
        }
    }
}

#if !SWIFT_PACKAGE
    #Preview {
        RootView()
    }
#endif
