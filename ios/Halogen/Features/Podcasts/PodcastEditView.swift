import SwiftUI

/// Edit a podcast's own fields (title / description / feed URL) — the web's
/// `/podcasts/:id/edit`.
struct PodcastEditView: View {
    let core: HalogenCore
    let podcast: PodcastData

    @Environment(\.dismiss) private var dismiss
    @State private var title: String
    @State private var description: String
    @State private var feedUrl: String
    @State private var error: String?
    @State private var saving = false

    init(core: HalogenCore, podcast: PodcastData) {
        self.core = core
        self.podcast = podcast
        _title = State(initialValue: podcast.title)
        _description = State(initialValue: podcast.description)
        _feedUrl = State(initialValue: podcast.feed_url)
    }

    var body: some View {
        Form {
            TextField("Title", text: $title)
            TextField("Feed URL", text: $feedUrl)
                .textContentType(.URL)
                .keyboardType(.URL)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Section("Description") {
                TextEditor(text: $description).frame(minHeight: 100)
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
                    // Online-only (feed-URL edits must surface server
                    // validation) — say so instead of failing raw (web
                    // podcast_edit.rs: submit disabled + labelled offline).
                    Text(core.isOffline ? "Offline — reconnect to save" : "Save")
                        .frame(maxWidth: .infinity)
                }
            }
            .disabled(
                saving || core.isOffline
                    || title.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .navigationTitle("Edit podcast")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func save() async {
        saving = true
        defer { saving = false }
        do {
            try await core.updatePodcast(
                id: podcast.id,
                title: title,
                description: description.isEmpty ? nil : description,
                feedUrl: feedUrl
            )
            await core.models?.podcasts.refresh()
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
