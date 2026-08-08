import SwiftUI

/// The library tab: every subscribed podcast, local-first, artwork via the
/// server art cache. Rows carry the quick-actions ellipsis (open is the row
/// tap); management pushes via state-driven destinations.
struct PodcastsView: View {
    @Bindable var model: PodcastsModel
    let core: HalogenCore

    @State private var showCreate = false
    @State private var manageTarget: PodcastManageTarget?
    @State private var deleteTarget: PodcastData?

    var body: some View {
        Group {
            if let error = model.error {
                LoadErrorView(title: "Couldn't load podcasts", message: error) {
                    await model.refresh()
                }
            } else if model.loaded && model.podcasts.isEmpty {
                ContentUnavailableView(
                    "No podcasts yet",
                    systemImage: "waveform.circle",
                    description: Text("Subscribe from Discover, or add a feed URL with +.")
                )
            } else if model.loaded && model.displayed.isEmpty {
                ContentUnavailableView.search(text: model.query.search)
            } else {
                List {
                    ForEach(model.displayed, id: \.id) { podcast in
                    HStack(spacing: 8) {
                        PodcastRow(podcast: podcast, artURL: core.podcastArtURL(podcast))
                        Spacer(minLength: 0)
                        Menu {
                            PodcastManageMenu(
                                manage: Binding(
                                    get: { manageTarget?.route },
                                    set: { route in
                                        manageTarget = route.map {
                                            PodcastManageTarget(podcastId: podcast.id, route: $0)
                                        }
                                    }
                                ),
                                confirmDelete: Binding(
                                    get: { deleteTarget?.id == podcast.id },
                                    set: { if $0 { deleteTarget = podcast } }
                                )
                            )
                        } label: {
                            Image(systemName: "ellipsis")
                                .foregroundStyle(.secondary)
                                .frame(width: 32, height: 32)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.borderless)
                    }
                    .background(
                        NavigationLink(value: AppRoute.podcast(podcast.id)) { EmptyView() }
                            .opacity(0)
                    )
                    // Long-press mirror of the ellipsis menu (same shared
                    // content — the two can't drift).
                    .contextMenu {
                        PodcastManageMenu(
                            manage: Binding(
                                get: { manageTarget?.route },
                                set: { route in
                                    manageTarget = route.map {
                                        PodcastManageTarget(podcastId: podcast.id, route: $0)
                                    }
                                }
                            ),
                            confirmDelete: Binding(
                                get: { deleteTarget?.id == podcast.id },
                                set: { if $0 { deleteTarget = podcast } }
                            )
                        )
                    }
                    }
                    // The sentinel pages the raw browse order; a live search
                    // shows every loaded match instead (web: `if !searching`).
                    if model.hasMore && !model.podcasts.isEmpty && model.query.search.isEmpty {
                        LoadMoreRow(failed: model.loadMoreFailed) { await model.loadMore() }
                    }
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            SortSearchBar(
                search: $model.query.search,
                field: $model.query.field,
                direction: $model.query.direction,
                fields: PodcastSortField.allCases.map { ($0, $0.label) },
                placeholder: "Search podcasts"
            )
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showCreate = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityIdentifier("podcast-add")
            }
        }
        .sheet(isPresented: $showCreate) {
            PodcastCreateSheet(core: core) { await model.refresh() }
        }
        .navigationDestination(item: $manageTarget) { target in
            if let podcast = model.podcasts.first(where: { $0.id == target.podcastId }) {
                PodcastManageScreen(route: target.route, core: core, podcast: podcast)
            }
        }
        .confirmationDialog(
            "Delete podcast?",
            isPresented: Binding(
                get: { deleteTarget != nil },
                set: { if !$0 { deleteTarget = nil } }
            )
        ) {
            if let target = deleteTarget {
                Button("Delete \(target.title)", role: .destructive) {
                    Task {
                        // Durable unsubscribe (optimistic + outbox) — offline-safe.
                        await core.unsubscribePodcast(id: target.id)
                    }
                }
            }
        } message: {
            Text("Removes the podcast, its episodes, and their server files.")
        }
        .task { await model.load() }
        .refreshable { await model.refresh() }
    }
}

/// A library-row manage push: which podcast + which screen.
struct PodcastManageTarget: Identifiable, Hashable {
    let podcastId: Int32
    let route: PodcastManageRoute

    var id: String { "\(podcastId)-\(route.rawValue)" }
}

/// One library row: artwork, title, author, episode count.
struct PodcastRow: View {
    let podcast: PodcastData
    let artURL: URL?

    var body: some View {
        HStack(spacing: 12) {
            Artwork(url: artURL, size: 56)
            VStack(alignment: .leading, spacing: 2) {
                Text(podcast.title)
                    .font(.headline)
                    .lineLimit(2)
                if let author = podcast.author, !author.isEmpty {
                    Text(author)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if let count = podcast.episode_count {
                    Text("\(count) episodes")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
        }
    }
}
