import SwiftUI

/// The Latest tab: newest episodes across every subscription, faceted by the
/// filter bar, local-first, with infinite scroll, restored scroll position,
/// configurable swipes, quick-action row menus, and bulk multiselect.
struct LatestView: View {
    @Bindable var model: LatestModel
    let core: HalogenCore

    @Environment(\.scenePhase) private var scenePhase
    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }

    var body: some View {
        content
            // safeAreaInset (not a plain VStack) so the bar keeps its own
            // layout slot under iOS 26's glass navbar instead of being
            // overlapped by its scroll-edge surface.
            .safeAreaInset(edge: .top, spacing: 0) {
                ListControlsBar(query: $model.query, allowsOnDevice: !core.isEmbeddedAccount)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if !selection.isEmpty {
                    BulkActionBar(
                        selection: $selection, selectedOnly: $selectedOnly,
                        episodes: model.episodes, core: core)
                }
            }
            .halogenNavbar(core: core)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    EditButton()
                }
            }
            .task(id: model.query) { await model.load() }
            .refreshable { await model.refresh() }
            // Leaving Edit closes multi-select for real (the bulk bar is
            // keyed on a non-empty selection).
            .onChange(of: isEditing) { _, editing in
                if !editing { selection.removeAll() }
            }
            // OnDevice facet: the list is a one-shot copy of the device set —
            // recompute when membership changes (a download completing or a
            // removal), or the facet shows stale rows until a manual refresh.
            .onChange(of: core.models?.device.onDevice.map(\.id) ?? []) {
                guard model.query.filters.contains(.onDevice) else { return }
                Task { await model.refresh() }
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active { model.persistScrollAnchor() }
            }
            .onDisappear { model.persistScrollAnchor() }
    }

    @ViewBuilder
    private var content: some View {
        if let error = model.error {
            ContentUnavailableView {
                Label("Couldn't load latest", systemImage: "wifi.exclamationmark")
            } description: {
                Text(error).font(.footnote.monospaced())
            }
        } else if model.loaded && model.episodes.isEmpty {
            ContentUnavailableView(
                "Nothing here",
                systemImage: "clock",
                description: Text(emptyHint)
            )
        } else {
            episodeList
        }
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? model.episodes.filter { selection.contains($0.id) } : model.episodes
    }

    private var episodeList: some View {
        ScrollViewReader { proxy in
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
                    .id(episode.id)
                    .configuredSwipes(.latest, episode: episode, core: core)
                    .onAppear { model.rowAppeared(episode) }
                    .onDisappear { model.rowDisappeared(episode) }
                }
                if model.hasMore && !model.episodes.isEmpty && !selectedOnly {
                    LoadMoreRow(failed: model.loadMoreFailed) { await model.loadMore() }
                }
            }
            .listStyle(.plain)
            .onChange(of: model.pendingScrollTo) { _, target in
                if let target {
                    proxy.scrollTo(target, anchor: .top)
                    model.pendingScrollTo = nil
                }
            }
        }
    }

    private var emptyHint: String {
        model.query == ListQuery()
            ? "New episodes land here after each feed poll."
            : "Nothing matches the current search/filter."
    }
}
