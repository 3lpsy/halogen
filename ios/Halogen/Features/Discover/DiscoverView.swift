import SwiftUI

/// Discover: online podcast search across the server's providers. Bare
/// results by design (no artwork — the server never proxies images here);
/// subscribing creates the podcast and the next poll ingests episodes.
/// Online-only, like the web.
struct DiscoverView: View {
    @Bindable var model: DiscoverModel
    let core: HalogenCore

    @State private var detail: DiscoverDetailBox?

    var body: some View {
        Group {
            if model.results.isEmpty && !model.searching {
                ContentUnavailableView(
                    model.query.isEmpty ? "Search podcasts" : "No results",
                    systemImage: "magnifyingglass",
                    description: Text(
                        model.query.isEmpty
                            ? "Search the server's providers. Searches need at least 2 characters."
                            : "Nothing matched \"\(model.query)\".")
                )
            } else {
                List(model.results, id: \.id) { item in
                    Button {
                        detail = DiscoverDetailBox(item: item)
                    } label: {
                        DiscoverRow(
                            item: item,
                            providerLabel: model.providerLabel(item.provider),
                            subscribed: model.isSubscribed(item),
                            subscribe: { Task { await model.subscribe(item) } }
                        )
                    }
                    .buttonStyle(.plain)
                }
                .listStyle(.plain)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            statusBar
        }
        .halogenNavbar(core: core)
        .searchable(text: $model.query, prompt: "Podcast name")
        .onSubmit(of: .search) {
            Task { await model.search() }
        }
        .overlay {
            if model.searching { ProgressView() }
        }
        .sheet(item: $detail) { box in
            DiscoverDetailSheet(item: box.item, model: model)
        }
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
        .task { await model.loadProviders() }
    }

    /// Offline banner / provider-fetch retry / provider toggle chips —
    /// the web Discover page's pre-results block.
    @ViewBuilder
    private var statusBar: some View {
        VStack(spacing: 6) {
            if model.isOffline {
                Label("Discover needs an internet connection.", systemImage: "wifi.slash")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if model.providerError && model.providers.isEmpty {
                HStack {
                    Text("Couldn't load the search providers.")
                        .font(.footnote)
                        .foregroundStyle(.red)
                    Spacer()
                    Button("Retry") { model.retryProviders() }
                        .font(.footnote)
                }
            }
            if !model.providers.isEmpty {
                HStack(spacing: 8) {
                    ForEach(model.providers, id: \.id) { info in
                        providerChip(info)
                    }
                    Spacer()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, model.providers.isEmpty && !model.isOffline && !model.providerError ? 0 : 8)
        .background(.bar)
    }

    /// One provider toggle chip: on = searched, off/unavailable = skipped.
    private func providerChip(_ info: DiscoverProviderInfo) -> some View {
        let on = info.available && model.isEnabled(info.id)
        return Button {
            model.toggleProvider(info.id)
        } label: {
            Text(info.label)
                .font(.caption.weight(.medium))
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(
                    Capsule().fill(on ? Color.accentColor.opacity(0.2) : Color(.secondarySystemFill))
                )
                .foregroundStyle(on ? Color.accentColor : .secondary)
        }
        .buttonStyle(.plain)
        .disabled(!info.available)
        .opacity(info.available ? 1 : 0.5)
    }
}

private struct DiscoverRow: View {
    let item: DiscoverResultItem
    let providerLabel: String
    let subscribed: Bool
    let subscribe: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title).font(.subheadline.weight(.medium))
                if let author = item.author, !author.isEmpty {
                    Text(author).font(.caption).foregroundStyle(.secondary)
                }
                if let description = item.description, !description.isEmpty {
                    Text(description)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                }
                Text(providerLabel)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            Button {
                subscribe()
            } label: {
                Image(systemName: subscribed ? "checkmark.circle.fill" : "plus.circle")
                    .font(.title3)
            }
            .buttonStyle(.plain)
            .foregroundStyle(subscribed ? .green : Color.accentColor)
            .disabled(subscribed)
        }
        .padding(.vertical, 2)
    }
}


/// Identifiable wrapper for sheet presentation.
struct DiscoverDetailBox: Identifiable {
    let item: DiscoverResultItem
    var id: String { item.id }
}

/// Per-result detail (the web's /discover/:id): full description, provider,
/// feed URL, subscribe.
struct DiscoverDetailSheet: View {
    let item: DiscoverResultItem
    let model: DiscoverModel

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text(item.title).font(.title3.bold())
                    if let author = item.author, !author.isEmpty {
                        Text(author).font(.subheadline).foregroundStyle(.secondary)
                    }
                    Text(model.providerLabel(item.provider))
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                    Text(item.feed_url)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    if let description = item.description, !description.isEmpty {
                        Divider()
                        Text(description).font(.callout)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
            }
            .navigationTitle("Podcast")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(model.isSubscribed(item) ? "Subscribed" : "Subscribe") {
                        Task {
                            await model.subscribe(item)
                            dismiss()
                        }
                    }
                    .disabled(model.isSubscribed(item))
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}
