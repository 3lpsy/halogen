import SwiftUI

/// Per-podcast download/poll config (web `/podcasts/:id/config/*`). The three
/// override fields are REQUIRED whole numbers, prefilled with the shared
/// defaults: the server's update can't clear a value to NULL, so the form
/// always sends a full set — reverting to defaults is the Remove action.
struct PodcastConfigFormView: View {
    /// The shared default constants (crates/utils constants.rs) — what the
    /// web prefills create forms and unset overrides with.
    private static let defaultPollInterval: UInt32 = 3600
    private static let defaultMaxEpisodes: UInt32 = 50
    private static let defaultMaxConcurrent: UInt32 = 3
    private static let defaultAutoDownload = false

    let core: HalogenCore
    let podcast: PodcastData

    @Environment(\.dismiss) private var dismiss
    @State private var pollInterval: String
    @State private var maxEpisodes: String
    @State private var maxConcurrent: String
    @State private var autoDownload: Bool
    @State private var error: String?
    @State private var saving = false
    /// The config body driving edit-vs-create — starts from the cached row,
    /// backfilled by id when the row carries only the FK.
    @State private var config: PodcastConfigData?
    /// Web `edit_ready`: a podcast whose cached row has a config FK but no
    /// body must NOT render as "create" prefilled with defaults — Save would
    /// overwrite the real server config. Held false until the body loads.
    @State private var editReady: Bool

    init(core: HalogenCore, podcast: PodcastData) {
        self.core = core
        self.podcast = podcast
        // Prefill with the shared defaults; on edit an existing override
        // wins, an unset field falls back to the default (web `fill`).
        let config = podcast.podcast_config
        _config = State(initialValue: config)
        _editReady = State(initialValue: podcast.podcast_config_id == nil || config != nil)
        _pollInterval = State(
            initialValue: String(config?.poll_interval_seconds ?? Self.defaultPollInterval))
        _maxEpisodes = State(
            initialValue: String(config?.max_episodes ?? Self.defaultMaxEpisodes))
        _maxConcurrent = State(
            initialValue: String(config?.max_concurrent_downloads ?? Self.defaultMaxConcurrent))
        _autoDownload = State(
            initialValue: config?.auto_download_enabled ?? Self.defaultAutoDownload)
    }

    var body: some View {
        Form {
            Section {
                field("Poll interval (secs)", text: $pollInterval, error: pollError)
                field("Max episodes", text: $maxEpisodes, error: maxEpisodesError)
                field("Max concurrent downloads", text: $maxConcurrent, error: maxConcurrentError)
                Toggle("Auto-download new episodes", isOn: $autoDownload)
            } footer: {
                Text(
                    "Override how often this podcast is polled and how its episodes download. Leave the defaults to match the server; Remove reverts to the server-wide defaults."
                )
            }

            if !editReady, error == nil {
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Loading current config…").foregroundStyle(.secondary)
                }
            }

            if let error {
                Text(error).font(.footnote).foregroundStyle(.red)
            }

            Button {
                Task { await save() }
            } label: {
                if saving {
                    ProgressView().frame(maxWidth: .infinity)
                } else {
                    Text("Save").frame(maxWidth: .infinity)
                }
            }
            .disabled(saving || !isValid || !editReady)

            if podcast.podcast_config_id != nil {
                Button("Remove config (use defaults)", role: .destructive) {
                    Task { await remove() }
                }
                .disabled(saving)
            }
        }
        .navigationTitle("Download config")
        .navigationBarTitleDisplayMode(.inline)
        .task { await backfillConfig() }
    }

    /// Cached row carries the FK but not the body → fetch it before the form
    /// is editable (web podcast_config_form.rs: prefill by id, `initialized`
    /// only on success).
    private func backfillConfig() async {
        guard !editReady, let configId = podcast.podcast_config_id else { return }
        do {
            let fetched = try await core.podcastConfig(id: configId)
            config = fetched
            // An unset override falls back to the shared default (web `fill`).
            pollInterval = String(fetched.poll_interval_seconds ?? Self.defaultPollInterval)
            maxEpisodes = String(fetched.max_episodes ?? Self.defaultMaxEpisodes)
            maxConcurrent = String(fetched.max_concurrent_downloads ?? Self.defaultMaxConcurrent)
            autoDownload = fetched.auto_download_enabled ?? Self.defaultAutoDownload
            editReady = true
            error = nil
        } catch {
            self.error =
                "Couldn't load the current config — editing is disabled so a save can't overwrite it. \(error)"
        }
    }

    private func field(_ label: String, text: Binding<String>, error: String?) -> some View {
        VStack(alignment: .trailing, spacing: 2) {
            LabeledContent(label) {
                TextField("", text: text).keyboardType(.numberPad)
                    .multilineTextAlignment(.trailing)
            }
            if let error {
                Text(error).font(.caption2).foregroundStyle(.red)
            }
        }
    }

    // MARK: - validation (web validate_fields: whole number + range rules
    // from PodcastConfigStoreData::validate)

    private static func parse(_ raw: String) -> UInt32? {
        let t = raw.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return nil }
        return UInt32(t)
    }

    private var pollError: String? {
        guard let v = Self.parse(pollInterval) else { return "Enter a whole number" }
        return v <= 86400 ? nil : "Poll interval must be between 0 and 86400 seconds"
    }

    private var maxEpisodesError: String? {
        guard let v = Self.parse(maxEpisodes) else { return "Enter a whole number" }
        return (1...10000).contains(v) ? nil : "Max episodes must be between 1 and 10000"
    }

    private var maxConcurrentError: String? {
        guard let v = Self.parse(maxConcurrent) else { return "Enter a whole number" }
        return (1...100).contains(v) ? nil : "Max concurrent downloads must be between 1 and 100"
    }

    private var isValid: Bool {
        pollError == nil && maxEpisodesError == nil && maxConcurrentError == nil
    }

    private func save() async {
        guard let p = Self.parse(pollInterval), let me = Self.parse(maxEpisodes),
            let mc = Self.parse(maxConcurrent), isValid
        else { return }
        saving = true
        defer { saving = false }
        do {
            if let config {
                // Always the FULL set — a nil field would mean "leave
                // unchanged" server-side, never "back to default".
                let data = PodcastConfigUpdateData(
                    poll_interval_seconds: p,
                    max_episodes: me,
                    max_concurrent_downloads: mc,
                    auto_download_enabled: autoDownload
                )
                // Web rule: edits go direct online (so the form shows server
                // errors) and queue as a durable UpdatePodcastConfig offline
                // (an existing id is safe to drain later).
                if core.isOffline {
                    await core.outbox?.enqueue(
                        .updatePodcastConfig(configId: config.id, data: data))
                    dismiss()
                    return
                }
                try await core.updatePodcastConfig(configId: config.id, data: data)
            } else {
                // Create stays online-only — it needs a real config id back
                // (web: the create submit is disabled offline).
                if core.isOffline {
                    error = "You're offline — reconnect to create a config."
                    return
                }
                try await core.createPodcastConfig(
                    podcastId: podcast.id,
                    data: PodcastConfigStoreData(
                        poll_interval_seconds: p,
                        max_episodes: me,
                        max_concurrent_downloads: mc,
                        auto_download_enabled: autoDownload
                    ))
            }
            await core.models?.podcasts.refresh()
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }

    private func remove() async {
        saving = true
        defer { saving = false }
        // Remove behaves like edit (an existing id): direct online, durable
        // RemovePodcastConfig op offline (web rule).
        if core.isOffline {
            await core.outbox?.enqueue(.removePodcastConfig(podcastId: podcast.id))
            dismiss()
            return
        }
        do {
            try await core.deletePodcastConfig(podcastId: podcast.id)
            await core.models?.podcasts.refresh()
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
