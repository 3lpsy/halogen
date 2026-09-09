import SwiftUI

struct DiscoverView: View {
    @Bindable var model: DiscoverModel
    let core: HalogenCore

    var body: some View {
        List {
            if model.mode == .podcast {
                ForEach(model.results, id: \.id) { item in
                    NavigationLink {
                        DiscoverPodcastView(item: item, model: model, core: core)
                    } label: {
                        VStack(alignment: .leading, spacing: 5) {
                            Text(item.title).font(.headline)
                            if let author = item.author { Text(author).font(.caption).foregroundStyle(.secondary) }
                            if let description = item.description, !description.isEmpty {
                                Text(HTMLText.preview(description)).font(.subheadline).foregroundStyle(.secondary)
                                    .lineLimit(3)
                            }
                            Text(model.providerLabel(item.provider)).font(.caption2).foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 4)
                    }
                }
            } else {
                ForEach(model.pages.episodes, id: \.id) { item in
                    NavigationLink {
                        DiscoverEpisodeView(item: item, model: model, core: core)
                    } label: {
                        DiscoverEpisodeRow(item: item)
                    }
                    .accessibilityIdentifier("discover-episode-\(item.title)")
                }
            }
            searchFooter
        }
        .accessibilityIdentifier("discover-results")
        .listStyle(.plain)
        .safeAreaInset(edge: .top, spacing: 0) { searchControls }
        .halogenNavbar(core: core)
        .task { await model.loadProviders() }
        .alert(
            "Search failed",
            isPresented: Binding(
                get: { model.error != nil },
                set: { if !$0 { model.clearError() } }
            )
        ) {
            Button("OK") { model.clearError() }
        } message: {
            Text(model.error ?? "")
        }
    }

    private var searchControls: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                TextField("Search", text: $model.query)
                    .textFieldStyle(.roundedBorder)
                    .submitLabel(.search)
                    .onSubmit { Task { await model.search() } }
                    .accessibilityLabel("Search Discover")
                    .disabled(model.isOffline || model.providers.isEmpty)
                Picker("Search by", selection: $model.mode) {
                    ForEach(DiscoverSearchMode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }
                .accessibilityIdentifier("discover-mode")
                .pickerStyle(.menu)
                .fixedSize()
            }
            if model.isOffline {
                Text("Discover needs an internet connection.").font(.footnote).foregroundStyle(.secondary)
            } else if model.providerError {
                HStack {
                    Text("Couldn't load search providers.").font(.footnote)
                    Button("Retry") { model.retryProviders() }
                }
            }
            HStack(spacing: 12) {
                ForEach(model.providers, id: \.id) { info in
                    Button {
                        model.toggleProvider(info.id)
                    } label: {
                        Label(info.label, systemImage: model.isEnabled(info.id) ? "checkmark.circle.fill" : "circle")
                            .font(.caption)
                    }
                    .disabled(!info.available || (model.mode == .episode && info.id == .gpodder))
                }
            }
            if model.mode == .episode {
                Text("gpodder supports podcast search only.").font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .background(.bar)
    }

    @ViewBuilder
    private var searchFooter: some View {
        if let error = model.pages.error {
            VStack(alignment: .leading, spacing: 8) {
                Text(error).font(.footnote).foregroundStyle(.red)
                HStack {
                    Button("Retry") { Task { await model.pages.loadMore() } }
                    Button("Search again") { Task { await model.search() } }
                }
            }
        } else if model.searching {
            HStack {
                Spacer(); ProgressView("Loading results…"); Spacer()
            }
        } else if model.pages.hasMore {
            HStack {
                Spacer(); ProgressView("Loading more…"); Spacer()
            }
            // Loading replaces this footer; the request must survive that view change.
            .onAppear { Task { await model.pages.loadMore() } }
        } else if model.pages.hasSearched {
            if model.results.isEmpty && model.pages.episodes.isEmpty {
                Text("No results found.").foregroundStyle(.secondary)
            }
            Text("Provider limit: up to \(model.pages.resultLimit) results per search.")
                .font(.caption).foregroundStyle(.secondary)
        } else {
            Text("Search podcasts or episodes. Enter at least 2 characters.")
                .foregroundStyle(.secondary)
        }
        ForEach(model.pages.providerErrors, id: \.provider) { error in
            Text("\(model.providerLabel(error.provider)): \(error.message)")
                .font(.footnote).foregroundStyle(.red)
        }
    }
}
