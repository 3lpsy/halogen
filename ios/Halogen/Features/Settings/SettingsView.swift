import SwiftUI

/// Settings: account + server info, dock configuration, sign out. Grows
/// toward the web's grouped settings pages (playback/downloads/UI per-account
/// preferences) as those features land natively.
struct SettingsView: View {
    let core: HalogenCore
    let nav: NavModel

    @State private var confirmSignOut = false

    var body: some View {
        List {
            // A discarded change must stay discoverable after the toast is
            // gone (web: the sync-failures affordance).
            if let failures = core.syncFailures, failures.count > 0 {
                Section {
                    NavigationLink {
                        SyncFailuresView(failures: failures)
                    } label: {
                        Label(
                            "\(failures.count) change\(failures.count == 1 ? "" : "s") couldn't sync",
                            systemImage: "exclamationmark.arrow.triangle.2.circlepath"
                        )
                        .foregroundStyle(.red)
                    }
                    .accessibilityIdentifier("settings-sync-failures")
                }
            }

            Section("Account") {
                if core.isEmbeddedAccount {
                    // Embedded credentials are app-managed (silent login) —
                    // renaming would desync the stored secrets and brick
                    // re-login (web: Edit is hidden for embedded accounts).
                    LabeledContent("User", value: core.account?.username ?? "—")
                } else {
                    NavigationLink {
                        UsernameEditView(core: core)
                    } label: {
                        LabeledContent("User", value: core.account?.username ?? "—")
                    }
                }
                LabeledContent(core.isEmbeddedAccount ? "Library" : "Server") {
                    Text(serverLabel).lineLimit(1).truncationMode(.middle)
                }
                LabeledContent("Status") {
                    HStack(spacing: 6) {
                        OnlineDot(status: core.connection.status)
                        Text(statusLabel)
                    }
                }
                NavigationLink("Accounts") {
                    AccountsView(core: core)
                }
            }

            if let prefs = core.models?.prefs {
                Section("UI") {
                    Picker(
                        "Size",
                        selection: Binding(
                            get: { prefs.prefs.uiSize },
                            set: { new in prefs.update { $0.uiSize = new } }
                        )
                    ) {
                        ForEach(ClientPrefs.UISize.allCases) { size in
                            Text(size.label).tag(size)
                        }
                    }
                    Picker(
                        "Theme",
                        selection: Binding(
                            get: { prefs.prefs.theme },
                            set: { new in prefs.update { $0.theme = new } }
                        )
                    ) {
                        ForEach(ClientPrefs.AppTheme.allCases) { theme in
                            Text(theme.label).tag(theme)
                        }
                    }
                }

                Section {
                    if core.isEmbeddedAccount {
                        LabeledContent("Playback source", value: "On-device library")
                    } else {
                        Picker(
                            "Playback source",
                            selection: Binding(
                                get: { prefs.prefs.playbackStrategy },
                                set: { new in prefs.update { $0.playbackStrategy = new } }
                            )
                        ) {
                            ForEach(ClientPrefs.PlaybackStrategy.allCases) { strategy in
                                Text(strategy.label).tag(strategy)
                            }
                        }
                    }
                    Picker(
                        "Skip forward",
                        selection: Binding(
                            get: { prefs.prefs.skipForwardSecs },
                            set: { new in prefs.update { $0.skipForwardSecs = new } }
                        )
                    ) {
                        ForEach([15, 30, 45, 60], id: \.self) { Text("\($0)s").tag($0) }
                    }
                    Picker(
                        "Skip back",
                        selection: Binding(
                            get: { prefs.prefs.skipBackSecs },
                            set: { new in prefs.update { $0.skipBackSecs = new } }
                        )
                    ) {
                        ForEach([10, 15, 30, 45], id: \.self) { Text("\($0)s").tag($0) }
                    }
                    Picker(
                        "Default speed",
                        selection: Binding(
                            get: { prefs.prefs.defaultRate },
                            set: { new in prefs.update { $0.defaultRate = new } }
                        )
                    ) {
                        ForEach(ClientPrefs.playbackRates, id: \.self) {
                            Text(String(format: "%g×", $0)).tag($0)
                        }
                    }
                    Toggle(
                        "Auto-play next in queue",
                        isOn: Binding(
                            get: { prefs.prefs.autoAdvance },
                            set: { new in prefs.update { $0.autoAdvance = new } }
                        )
                    )
                    Toggle(
                        "Next/previous track buttons skip within the episode (for Bluetooth devices without seek buttons)",
                        isOn: Binding(
                            get: { prefs.prefs.mediaNextPrevSeek },
                            set: { new in prefs.update { $0.mediaNextPrevSeek = new } }
                        )
                    )
                    Toggle(
                        "Add to front of queue",
                        isOn: Binding(
                            get: { prefs.prefs.addToQueueFront },
                            set: { new in prefs.update { $0.addToQueueFront = new } }
                        )
                    )
                    Picker(
                        "Sleep timer",
                        selection: Binding(
                            get: { prefs.prefs.defaultSleepMinutes },
                            set: { new in prefs.update { $0.defaultSleepMinutes = new } }
                        )
                    ) {
                        ForEach(ClientPrefs.sleepDurations, id: \.self) {
                            Text("\($0) min").tag($0)
                        }
                    }
                    Toggle(
                        "Sleep timer by default",
                        isOn: Binding(
                            get: { prefs.prefs.sleepByDefault },
                            set: { new in prefs.update { $0.sleepByDefault = new } }
                        )
                    )
                } header: {
                    Text("Playback")
                } footer: {
                    if core.isEmbeddedAccount {
                        Text("Local Only plays from the on-device library. Downloaded audio stays on this device.")
                    }
                }

                // Embedded accounts have no device downloads to tune — the
                // media already lives in the on-device server (web:
                // settings/downloads.rs replaces the page with that notice).
                if !core.isEmbeddedAccount {
                    Section {
                        Picker(
                            "Chunk size",
                            selection: Binding(
                                get: { prefs.prefs.downloadChunkKiB },
                                set: { new in prefs.update { $0.downloadChunkKiB = new } }
                            )
                        ) {
                            ForEach(ClientPrefs.downloadChunkKiBOptions, id: \.self) { kib in
                                Text(kib == 0 ? "No chunking (whole file)" : "\(kib / 1024) MB")
                                    .tag(kib)
                            }
                        }
                        Picker(
                            "Parallel chunks",
                            selection: Binding(
                                get: { prefs.prefs.downloadParallelism },
                                set: { new in prefs.update { $0.downloadParallelism = new } }
                            )
                        ) {
                            ForEach(ClientPrefs.downloadParallelisms, id: \.self) {
                                Text("\($0)").tag($0)
                            }
                        }
                        // No chunking = a single request — nothing to
                        // parallelize (web: the select is disabled too).
                        .disabled(prefs.prefs.downloadChunkKiB == 0)
                    } header: {
                        Text("Downloads")
                    } footer: {
                        Text(
                            "Device downloads fetch in chunks — each finished chunk is saved progress, so slow or flaky "
                                + "connections resume instead of restarting. Parallel chunks fetch concurrently within one download."
                        )
                    }
                }
            }

            Section("Navigation") {
                NavigationLink("Configure dock") {
                    ConfigureDockView(nav: nav, isAdmin: core.isAdmin)
                }
                if let swipes = core.models?.swipes {
                    NavigationLink("Configure swipes") {
                        ConfigureSwipesView(swipes: swipes)
                    }
                }
            }

            // Embedded accounts sign in silently with app-managed secrets —
            // a user-set password would desync them (web hides the whole
            // Edit surface, password included, for embedded accounts).
            if !core.isEmbeddedAccount {
                Section("Security") {
                    NavigationLink("Change password") {
                        ChangePasswordView(core: core)
                    }
                }
            }

            // Admin-only server surfaces (web: settings/server.rs `if
            // is_admin()` wraps DB transfer/config/errors, and the OPML page
            // is gated the same way on the settings menu) — non-admins would
            // only hit 403s here.
            if core.isAdmin {
                Section("Server") {
                    NavigationLink("Server errors") {
                        ServerErrorsView(core: core)
                    }
                    NavigationLink("View config") {
                        JSONKeyValueView(title: "Server config", core: core, path: "admin/config")
                    }
                    NavigationLink("Config overrides") {
                        ConfigOverridesView(core: core)
                    }
                    NavigationLink("OPML import / export") {
                        OpmlView(core: core)
                    }
                    NavigationLink("Database export / import") {
                        DbTransferView(core: core)
                    }
                }
            }

            Section("Local data") {
                NavigationLink("Storage & purge") {
                    LocalDataView(core: core)
                }
            }

            Section {
                Button("Sign out", role: .destructive) {
                    confirmSignOut = true
                }
            } footer: {
                Text("Cached data stays on this device and reappears when the same account signs in again.")
            }
        }
        .halogenNavbar(core: core)
        .confirmationDialog("Sign out?", isPresented: $confirmSignOut) {
            Button("Sign out", role: .destructive) {
                core.signOut()
            }
        } message: {
            Text("You'll return to the landing page.")
        }
    }

    private var serverLabel: String {
        switch core.account?.kind {
        case .embedded: return "Local Only"
        case .remote(let url): return url
        case nil: return "—"
        }
    }

    private var statusLabel: String {
        switch core.connection.status {
        case .online: return "Online"
        case .offline: return "Offline"
        case .unknown: return "Checking…"
        }
    }
}

/// Configure the dock: reorder destinations and toggle visibility — the
/// web's `/settings/dock` rules: first five visible items form the dock, the
/// More slot is always present, Settings can't be hidden.
struct ConfigureDockView: View {
    @Bindable var nav: NavModel
    /// Non-admins never see the admin-only rows (Polling, Server Logs) —
    /// web: `configurable_order` filters via `nav_label(key, admin)`.
    let isAdmin: Bool

    /// The rows this user can see and manage, in stored order — the page's
    /// working copy (web: dock_config.rs `configurable_order`).
    private var configurableOrder: [BuiltinNav] {
        nav.config.order.filter { isAdmin || !$0.adminOnly }
    }

    var body: some View {
        List {
            Section {
                ForEach(configurableOrder) { item in
                    HStack {
                        Image(systemName: item.systemImage)
                            .frame(width: 28)
                            .foregroundStyle(.secondary)
                        Text(item.label)
                        Spacer()
                        Toggle(
                            "",
                            isOn: Binding(
                                get: { !nav.config.hidden.contains(item) },
                                set: { visible in
                                    var config = nav.config
                                    if visible {
                                        config.hidden.removeAll { $0 == item }
                                    } else if item != .settings {
                                        config.hidden.append(item)
                                    }
                                    nav.update(config)
                                }
                            )
                        )
                        .labelsHidden()
                        .accessibilityIdentifier("nav-toggle-\(item.rawValue)")
                        .disabled(item == .settings)
                    }
                }
                .onMove { from, to in
                    // Reorder the visible working copy, carrying unmanageable
                    // rows (admin-only keys for a non-admin) through in stored
                    // order (web `merge_nav_keys`) — a drag never drops them.
                    var working = configurableOrder
                    working.move(fromOffsets: from, toOffset: to)
                    var config = nav.config
                    config.order = working + config.order.filter { !working.contains($0) }
                    nav.update(config)
                }
            } footer: {
                Text(
                    "Drag to reorder. The dock shows the first \(NavModel.dockSlots) visible items; everything visible appears in More. Settings can't be hidden."
                )
            }
        }
        .navigationTitle("Configure dock")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                EditButton()
            }
        }
    }
}
