import SwiftUI

/// The Downloads tab: server-held episode files, Downloaded/Downloading
/// facets, swipe to delete the server file.
struct DownloadsView: View {
    @Bindable var model: DownloadsModel
    let core: HalogenCore

    private struct TaskKey: Equatable {
        let facet: DownloadsModel.Facet
        let query: ListQuery
    }

    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }

    /// Completed+failed server-download ids — the server facets' reactivity
    /// signal (a single Equatable value keeps the type-checker happy).
    private var serverOutcomes: [Int32] {
        let downloads = core.models?.serverDownloads
        return (downloads?.completed ?? []).sorted() + (downloads?.failed ?? []).sorted()
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? model.episodes.filter { selection.contains($0.id) } : model.episodes
    }

    var body: some View {
        Group {
            if let error = model.error {
                LoadErrorView(title: "Couldn't load downloads", message: error) {
                    await model.refresh()
                }
            } else if model.loaded && model.episodes.isEmpty {
                if !model.query.search.isEmpty || !model.query.filters.isEmpty {
                    ContentUnavailableView(
                        "No matches",
                        systemImage: "line.3.horizontal.decrease.circle",
                        description: Text("No downloads match the current search or filters.")
                    )
                } else {
                    ContentUnavailableView(
                        "No \(model.facet.label.lowercased()) episodes",
                        systemImage: "arrow.down.circle",
                        description: Text("Trigger downloads from an episode's context menu.")
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
                        .configuredSwipes(.downloads, episode: episode, core: core)
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                Task { await model.removeDownload(episode) }
                            } label: {
                                Label(
                                    model.facet == .onDevice ? "Remove" : "Delete file",
                                    systemImage: "trash")
                            }
                        }
                    }
                    if model.hasMore && !model.episodes.isEmpty && !selectedOnly {
                        LoadMoreRow(failed: model.loadMoreFailed) { await model.loadMore() }
                    }
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                Picker("Facet", selection: $model.facet) {
                    ForEach(model.availableFacets) { facet in
                        Text(facet.label).tag(facet)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.top, 8)
                ListControlsBar(query: $model.query, showFilter: false)
            }
            .background(.bar)
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
        .task(id: TaskKey(facet: model.facet, query: model.query)) { await model.load() }
        .refreshable { await model.refresh() }
        // Leaving Edit closes multi-select for real (the bulk bar is
        // keyed on a non-empty selection).
        .onChange(of: isEditing) { _, editing in
            if !editing { selection.removeAll() }
        }
        // On-device facet reactivity: recompute when the device set changes
        // (see LatestView's identical hook).
        .onChange(of: core.models?.device.onDevice.map(\.id) ?? []) {
            guard model.facet == .onDevice else { return }
            Task { await model.refresh() }
        }
        // Server facets: a download finishing (or failing) while the user
        // watches must move the row between Downloading/Server live.
        .onChange(of: serverOutcomes) {
            guard model.facet != .onDevice else { return }
            Task { await model.refresh() }
        }
    }
}
