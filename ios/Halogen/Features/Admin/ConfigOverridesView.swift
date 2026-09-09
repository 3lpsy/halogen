import SwiftUI

/// Config-overrides editor (admin): typed editing of the allowlisted
/// parameters (wire ConfigOverridesData). POST replaces the set wholesale;
/// changes apply after a server restart — the web's config-overrides page
/// (webui/views config_overrides_form) key-for-key.
struct ConfigOverridesView: View {
    /// The input a parameter renders as (web: InputKind).
    enum ParamKind {
        case number, percent, bool, date, text
    }

    /// Static metadata for one overridable parameter — `key` mirrors the
    /// ConfigOverridesData field name exactly.
    struct ParamMeta: Identifiable {
        let key: String
        let label: String
        let desc: String
        let kind: ParamKind

        var id: String { key }
    }

    /// One editable override row in the working set.
    struct Row: Identifiable {
        let key: String
        var value: String
        var error: String?

        var id: String { key }
    }

    /// The full overridable allowlist (mirrors the web's PARAMS registry).
    static let params: [ParamMeta] = [
        ParamMeta(
            key: "subscription_fallback_poll_interval_secs",
            label: "Fallback poll interval (secs)", desc: "Default feed poll cadence",
            kind: .number),
        ParamMeta(
            key: "subscription_poll_wake_interval_secs",
            label: "Poll wake interval (secs)", desc: "How often the poller wakes",
            kind: .number),
        ParamMeta(
            key: "subscription_fallback_max_episodes",
            label: "Fallback max episodes", desc: "Max episodes fetched per feed",
            kind: .number),
        ParamMeta(
            key: "subscription_max_concurrent_downloads",
            label: "Max concurrent downloads", desc: "Parallel episode downloads",
            kind: .number),
        ParamMeta(
            key: "subscription_max_poll_concurrent",
            label: "Max concurrent polls", desc: "Parallel feed polls",
            kind: .number),
        ParamMeta(
            key: "subscription_poll_auto_download_enabled",
            label: "Auto-download on poll", desc: "Server-side auto-download default",
            kind: .bool),
        ParamMeta(
            key: "subscription_auto_playlist_add_to_start",
            label: "Auto-add to start of playlists",
            desc: "Insert auto-added episodes at the start", kind: .bool),
        ParamMeta(
            key: "subscription_no_sync_before",
            label: "No sync before", desc: "Ignore episodes before this date",
            kind: .date),
        ParamMeta(
            key: "subscription_sync_on_start",
            label: "Sync on start", desc: "Sync feeds at server boot", kind: .bool),
        ParamMeta(
            key: "auth_token_expiry_minutes",
            label: "Token expiry (mins)", desc: "JWT lifetime in minutes", kind: .number),
        ParamMeta(
            key: "episode_playback_complete_percentage",
            label: "Playback complete %", desc: "Mark finished within the last N%",
            kind: .percent),
        ParamMeta(
            key: "opml_file",
            label: "OPML file", desc: "Server path to a seed OPML file", kind: .text),
    ]

    let core: HalogenCore

    @Environment(\.dismiss) private var dismiss
    @State private var rows: [Row] = []
    @State private var query = ""
    @State private var loaded = false
    @State private var loadError: String?
    @State private var serverError: String?
    @State private var overridesDisabled = false
    @State private var busy = false
    @State private var confirmSave = false
    @State private var confirmClear = false

    private var mutateDisabled: Bool {
        busy || core.isOffline || overridesDisabled
    }

    var body: some View {
        List {
            if overridesDisabled {
                Section {
                    Text("Config overrides are disabled on this server; changes can't be saved.")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }
            }
            if let loadError {
                Section {
                    Text("Could not load overrides: \(loadError)")
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            } else {
                addSection
                activeSection

                if let serverError {
                    Section {
                        Text(serverError).font(.footnote).foregroundStyle(.red)
                    }
                }

                Section {
                    Button("Save overrides") {
                        serverError = nil
                        if validate() { confirmSave = true }
                    }
                    .disabled(mutateDisabled)
                    Button("Clear all", role: .destructive) {
                        serverError = nil
                        confirmClear = true
                    }
                    .disabled(mutateDisabled)
                } footer: {
                    Text("Overrides are saved to the server's overrides file and take effect after a restart.")
                }
            }
        }
        .navigationTitle("Config overrides")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .refreshable {
            loaded = false
            await load()
        }
        .confirmationDialog(
            "Save config overrides?", isPresented: $confirmSave, titleVisibility: .visible
        ) {
            Button("Save") { Task { await save() } }
        } message: {
            Text(
                "This replaces the server's overrides file with the current set. Restart the server afterwards to apply them."
            )
        }
        .confirmationDialog(
            "Clear all overrides?", isPresented: $confirmClear, titleVisibility: .visible
        ) {
            Button("Clear all", role: .destructive) { Task { await clearAll() } }
        } message: {
            Text(
                "This deletes every override and reverts the server to its configured defaults. Restart afterwards to apply."
            )
        }
    }

    // MARK: - sections

    private var suggestions: [ParamMeta] {
        let active = Set(rows.map(\.key))
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return Self.params.filter { p in
            guard !active.contains(p.key) else { return false }
            return q.isEmpty || p.label.lowercased().contains(q)
                || p.desc.lowercased().contains(q) || p.key.contains(q)
        }
    }

    @ViewBuilder
    private var addSection: some View {
        Section("Add an override") {
            TextField("Search parameters…", text: $query)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            if suggestions.isEmpty {
                Text("No matching parameters.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(suggestions) { p in
                    Button {
                        // Booleans default to Enabled (a valid value);
                        // others start empty (web parity).
                        rows.append(
                            Row(key: p.key, value: p.kind == .bool ? "true" : "", error: nil))
                        query = ""
                    } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(p.label).foregroundStyle(.primary)
                            Text(p.desc).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var activeSection: some View {
        Section {
            if rows.isEmpty {
                Text("No overrides set yet — search above to add one.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            ForEach($rows) { row in
                OverrideRowView(
                    meta: Self.params.first { $0.key == row.wrappedValue.key },
                    row: row)
            }
            .onDelete { offsets in
                rows.remove(atOffsets: offsets)
            }
        } header: {
            Text("Active overrides")
        } footer: {
            if !rows.isEmpty {
                Text("Swipe to remove one, then Save to apply the new set.")
            }
        }
    }

    // MARK: - load / save

    private func load() async {
        guard !loaded else { return }
        do {
            let data = try await core.configOverrides()
            rows = Self.params.compactMap { p in
                Self.currentValue(data, key: p.key).map {
                    Row(key: p.key, value: $0, error: nil)
                }
            }
            loaded = true
            loadError = nil
        } catch {
            loadError = FriendlyError.message(error)
        }
        // Best-effort: proactively learn whether the mechanism is disabled.
        if let cfg = try? await core.rawJSON("admin/config"),
            let disabled = cfg["config_overrides_disabled"] as? Bool
        {
            overridesDisabled = disabled
        }
    }

    /// Parse every row into typed data; annotate rows with inline errors.
    private func validate() -> Bool {
        var data = ConfigOverridesData()
        var ok = true
        for i in rows.indices {
            if let err = Self.applyValue(&data, key: rows[i].key, raw: rows[i].value) {
                rows[i].error = err
                ok = false
            } else {
                rows[i].error = nil
            }
        }
        return ok
    }

    private func save() async {
        var data = ConfigOverridesData()
        for row in rows {
            guard Self.applyValue(&data, key: row.key, raw: row.value) == nil else { return }
        }
        busy = true
        defer { busy = false }
        do {
            try await core.setConfigOverrides(data)
            serverError = nil
            ToastCenter.shared.success(
                "Overrides saved — restart the server for them to take effect.")
            dismiss()
        } catch {
            serverError = FriendlyError.message(error)
        }
    }

    private func clearAll() async {
        busy = true
        defer { busy = false }
        do {
            try await core.clearConfigOverrides()
            rows = []
            serverError = nil
            ToastCenter.shared.success("All overrides cleared — restart the server to apply.")
            dismiss()
        } catch {
            serverError = FriendlyError.message(error)
        }
    }

    // MARK: - typed value (de)serialization (web: params.rs)

    /// The current (set) value of `key` in `data`, stringified, or nil if unset.
    static func currentValue(_ data: ConfigOverridesData, key: String) -> String? {
        switch key {
        case "subscription_fallback_poll_interval_secs":
            return data.subscription_fallback_poll_interval_secs.map { String($0) }
        case "subscription_poll_wake_interval_secs":
            return data.subscription_poll_wake_interval_secs.map { String($0) }
        case "subscription_fallback_max_episodes":
            return data.subscription_fallback_max_episodes.map { String($0) }
        case "subscription_max_concurrent_downloads":
            return data.subscription_max_concurrent_downloads.map { String($0) }
        case "subscription_max_poll_concurrent":
            return data.subscription_max_poll_concurrent.map { String($0) }
        case "subscription_poll_auto_download_enabled":
            return data.subscription_poll_auto_download_enabled.map { $0 ? "true" : "false" }
        case "subscription_auto_playlist_add_to_start":
            return data.subscription_auto_playlist_add_to_start.map { $0 ? "true" : "false" }
        case "subscription_no_sync_before":
            return data.subscription_no_sync_before
        case "subscription_sync_on_start":
            return data.subscription_sync_on_start.map { $0 ? "true" : "false" }
        case "auth_token_expiry_minutes":
            return data.auth_token_expiry_minutes.map { String($0) }
        case "episode_playback_complete_percentage":
            return data.episode_playback_complete_percentage.map { String($0) }
        case "opml_file":
            return data.opml_file
        default:
            return nil
        }
    }

    /// Parse `raw` and set it into `data` under `key`; returns a short inline
    /// error message on a parse/range failure.
    static func applyValue(
        _ data: inout ConfigOverridesData, key: String, raw: String
    ) -> String? {
        let raw = raw.trimmingCharacters(in: .whitespaces)
        let whole = "Enter a whole number"
        switch key {
        case "subscription_fallback_poll_interval_secs":
            guard let v = UInt64(raw) else { return whole }
            data.subscription_fallback_poll_interval_secs = v
        case "subscription_poll_wake_interval_secs":
            guard let v = UInt64(raw) else { return whole }
            data.subscription_poll_wake_interval_secs = v
        case "subscription_fallback_max_episodes":
            guard let v = UInt(raw) else { return whole }
            data.subscription_fallback_max_episodes = v
        case "subscription_max_concurrent_downloads":
            guard let v = UInt(raw) else { return whole }
            data.subscription_max_concurrent_downloads = v
        case "subscription_max_poll_concurrent":
            guard let v = UInt(raw) else { return whole }
            data.subscription_max_poll_concurrent = v
        case "subscription_poll_auto_download_enabled":
            guard let v = parseBool(raw) else { return "Choose enabled or disabled" }
            data.subscription_poll_auto_download_enabled = v
        case "subscription_auto_playlist_add_to_start":
            guard let v = parseBool(raw) else { return "Choose enabled or disabled" }
            data.subscription_auto_playlist_add_to_start = v
        case "subscription_no_sync_before":
            guard isDate(raw) else { return "Use YYYY-MM-DD" }
            data.subscription_no_sync_before = raw
        case "subscription_sync_on_start":
            guard let v = parseBool(raw) else { return "Choose enabled or disabled" }
            data.subscription_sync_on_start = v
        case "auth_token_expiry_minutes":
            guard let v = UInt64(raw) else { return whole }
            data.auth_token_expiry_minutes = v
        case "episode_playback_complete_percentage":
            guard let v = UInt64(raw) else { return whole }
            guard v <= 100 else { return "Must be between 0 and 100" }
            data.episode_playback_complete_percentage = UInt16(v)
        case "opml_file":
            guard !raw.isEmpty else { return "Enter a path" }
            data.opml_file = raw
        default:
            return "Unknown parameter"
        }
        return nil
    }

    private static func parseBool(_ raw: String) -> Bool? {
        switch raw {
        case "true": return true
        case "false": return false
        default: return nil
        }
    }

    /// Lenient `YYYY-MM-DD` shape check (the server re-parses).
    private static func isDate(_ raw: String) -> Bool {
        let bytes = Array(raw.utf8)
        guard bytes.count == 10 else { return false }
        for (i, b) in bytes.enumerated() {
            if i == 4 || i == 7 {
                if b != UInt8(ascii: "-") { return false }
            } else if !(UInt8(ascii: "0")...UInt8(ascii: "9")).contains(b) {
                return false
            }
        }
        return true
    }
}

/// One active override: label/description + the type-appropriate input + the
/// inline parse error.
private struct OverrideRowView: View {
    let meta: ConfigOverridesView.ParamMeta?
    @Binding var row: ConfigOverridesView.Row

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(meta?.label ?? row.key)
                .font(.subheadline.weight(.medium))
            if let desc = meta?.desc, !desc.isEmpty {
                Text(desc).font(.caption).foregroundStyle(.secondary)
            }
            input
            if let error = row.error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder
    private var input: some View {
        switch meta?.kind ?? .text {
        case .bool:
            Toggle(
                "Enabled",
                isOn: Binding(
                    get: { row.value == "true" },
                    set: {
                        row.value = $0 ? "true" : "false"; row.error = nil
                    }
                )
            )
            .font(.callout)
        case .number, .percent:
            TextField("0", text: valueBinding)
                .keyboardType(.numberPad)
                .font(.callout.monospaced())
        case .date:
            TextField("YYYY-MM-DD", text: valueBinding)
                .keyboardType(.numbersAndPunctuation)
                .autocorrectionDisabled()
                .font(.callout.monospaced())
        case .text:
            TextField("Value", text: valueBinding)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .font(.callout.monospaced())
        }
    }

    private var valueBinding: Binding<String> {
        Binding(
            get: { row.value },
            set: {
                row.value = $0; row.error = nil
            }
        )
    }
}
