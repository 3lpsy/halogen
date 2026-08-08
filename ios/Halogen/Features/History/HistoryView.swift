import SwiftUI

/// History: recently played episodes (in-progress + finished), most recent
/// playback first — the web sorts its local pool by playback recency; here
/// the two playback facets are fetched and merged, cached for offline.
struct HistoryView: View {
    @Bindable var model: HistoryModel
    let core: HalogenCore

    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? model.episodes.filter { selection.contains($0.id) } : model.episodes
    }

    var body: some View {
        Group {
            if let error = model.error {
                LoadErrorView(title: "Couldn't load history", message: error) {
                    await model.refresh()
                }
            } else if model.loaded && model.episodes.isEmpty {
                // "Nothing played yet" over a filtered/searched view read as
                // a wiped history — distinguish no-match from truly empty.
                if !model.query.search.isEmpty || !model.query.filters.isEmpty {
                    ContentUnavailableView(
                        "No matches",
                        systemImage: "line.3.horizontal.decrease.circle",
                        description: Text("No played episodes match the current search or filters.")
                    )
                } else {
                    ContentUnavailableView(
                        "Nothing played yet",
                        systemImage: "clock.arrow.circlepath",
                        description: Text("Episodes you play appear here.")
                    )
                }
            } else {
                List(selection: $selection) {
                    ForEach(displayedEpisodes, id: \.id) { episode in
                        EpisodeRowLink(
                            episode: episode,
                            artURL: core.episodeArtURL(episode),
                            subtitle: episode.podcast?.title,
                            context: .browse,
                            core: core
                        )
                        .tag(episode.id)
                        .selectionDisabled(!isEditing)
                        .configuredSwipes(.history, episode: episode, core: core)
                    }
                    if model.hasMore && !model.episodes.isEmpty && !selectedOnly {
                        LoadMoreRow(failed: model.loadMoreFailed) { await model.loadMore() }
                    }
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            // Full filter set on History (web parity) — the chips AND with
            // the intrinsic played/finished membership.
            ListControlsBar(
                query: $model.query, allowsRecency: true,
                allowsOnDevice: !core.isEmbeddedAccount)
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                EditButton()
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !selection.isEmpty {
                BulkActionBar(
                    selection: $selection, selectedOnly: $selectedOnly,
                    episodes: model.episodes, core: core)
            }
        }
        .task(id: model.query) { await model.load() }
        .refreshable { await model.refresh() }
        // Leaving Edit closes multi-select for real (the bulk bar is
        // keyed on a non-empty selection).
        .onChange(of: isEditing) { _, editing in
            if !editing { selection.removeAll() }
        }
    }
}
