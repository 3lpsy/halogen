import SwiftUI

/// Subscribe by feed URL (the web's `/podcasts/create`). The next poll
/// ingests episodes; Discover is the search-first alternative.
struct PodcastCreateSheet: View {
    let core: HalogenCore
    let onCreated: () async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var title = ""
    @State private var feedUrl = ""
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        NavigationStack {
            Form {
                TextField("Title", text: $title)
                TextField("Feed URL (https://…)", text: $feedUrl)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                if let error {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
            .navigationTitle("Add podcast")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        Task { await save() }
                    }
                    .accessibilityIdentifier("podcast-create-submit")
                    // Title optional (falls back to the feed URL until the
                    // first ingest heals it — web rule); the URL must at
                    // least parse as http(s), or the durable op dead-letters
                    // silently after the user has moved on.
                    .disabled(saving || !feedUrlValid)
                }
            }
        }
        .presentationDetents([.medium])
    }

    private var feedUrlValid: Bool {
        let trimmed = feedUrl.trimmingCharacters(in: .whitespaces)
        guard let url = URL(string: trimmed), let scheme = url.scheme?.lowercased(),
            scheme == "http" || scheme == "https", url.host != nil
        else { return false }
        return true
    }

    private func save() async {
        saving = true
        defer { saving = false }
        // Durable subscribe (web: OutboxOp::Subscribe) — queues offline; the
        // podcast appears once the op drains and the library refreshes. An
        // empty title rides as nil; the drain falls back to the feed URL.
        let trimmedTitle = title.trimmingCharacters(in: .whitespaces)
        guard
            await core.ensureQueued(
                .subscribe(
                    feedUrl: feedUrl.trimmingCharacters(in: .whitespaces),
                    title: trimmedTitle.isEmpty ? nil : trimmedTitle,
                    description: nil
                ))
        else { return }
        await onCreated()
        dismiss()
    }
}
