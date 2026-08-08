import SwiftUI

/// The Queue tab: the default playlist in position order. Drag to reorder
/// (Edit mode), swipe to remove — both offline-capable (optimistic + outbox).
struct QueueView: View {
    @Bindable var model: QueueModel
    let core: HalogenCore

    @State private var selection = Set<Int32>()
    @State private var selectedOnly = false
    /// Row selection is an EDIT-MODE tool only: iOS 17 lists tap-select
    /// outside edit mode too, which hijacked plain row taps into an
    /// unclosable multi-select. Rows disable selection until Edit is active,
    /// and leaving Edit clears the set (closes the bulk bar).
    @Environment(\.editMode) private var editMode

    private var isEditing: Bool { editMode?.wrappedValue.isEditing == true }

    var body: some View {
        Group {
            if let error = model.error {
                // Cold offline start with nothing cached gets the web's
                // dedicated tri-state copy, not a raw URLError.
                if core.isOffline {
                    ContentUnavailableView {
                        Label("Queue unavailable offline", systemImage: "wifi.slash")
                    } description: {
                        Text("Reconnect once to load it — afterwards the queue stays available offline.")
                    }
                } else {
                    LoadErrorView(title: "Couldn't load queue", message: error) {
                        await model.refresh()
                    }
                }
            } else if model.loaded && model.queue == nil {
                noQueue
            } else if model.loaded && model.displayed.isEmpty {
                ContentUnavailableView(
                    model.episodes.isEmpty ? "Queue is empty" : "No matches",
                    systemImage: "list.bullet",
                    description: Text("Add episodes from any list via their context menu.")
                )
            } else {
                queueList
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            ListControlsBar(query: $model.query, allowsPosition: true)
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
                    episodes: model.displayed, core: core, context: .queue)
            }
        }
        .task { await model.load() }
        .refreshable { await model.refresh() }
        // Leaving Edit closes multi-select for real (the bulk bar is
        // keyed on a non-empty selection).
        .onChange(of: isEditing) { _, editing in
            if !editing { selection.removeAll() }
        }
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    private var displayedEpisodes: [EpisodeData] {
        selectedOnly ? model.displayed.filter { selection.contains($0.id) } : model.displayed
    }

    private var queueList: some View {
        List(selection: $selection) {
            ForEach(displayedEpisodes, id: \.id) { episode in
                EpisodeRowLink(
                    episode: episode,
                    artURL: core.episodeArtURL(episode),
                    subtitle: episode.podcast?.title,
                    context: .queue,
                    core: core
                )
                .tag(episode.id)
                .selectionDisabled(!isEditing)
                .configuredSwipes(.queue, episode: episode, core: core, context: .queue)
                .swipeActions(edge: .trailing) {
                    Button(role: .destructive) {
                        model.remove(episode)
                    } label: {
                        Label("Remove", systemImage: "minus.circle")
                    }
                }
            }
            .onMove { from, to in
                // selectedOnly shows a SUBSET — its indices don't map onto the
                // full array, and a drag would silently reorder hidden rows
                // and sync that corruption to the server.
                guard model.reorderable, !selectedOnly else { return }
                model.move(fromOffsets: from, toOffset: to)
            }
            .moveDisabled(!model.reorderable || selectedOnly)
        }
        .listStyle(.plain)
    }

    /// Mirrors the web's "No queue yet" page: create the default playlist.
    private var noQueue: some View {
        ContentUnavailableView {
            Label("No queue yet", systemImage: "list.bullet")
        } description: {
            Text("The queue is your default playlist — create it to start queueing episodes.")
        } actions: {
            // Online-only: the created id anchors every later offline op
            // (web: the create form's submit is disabled offline).
            Button(core.isOffline ? "Create queue (offline)" : "Create queue") {
                Task { await model.createQueue() }
            }
            .buttonStyle(.borderedProminent)
            .disabled(core.isOffline)
        }
    }
}
