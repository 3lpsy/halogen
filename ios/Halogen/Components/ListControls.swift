import SwiftUI

/// Sort field vocabulary for episode lists (server `order[order_by]` names).
/// `.position` is the queue's pivot order (local, queue-only).
enum EpisodeOrderField: String, Codable, CaseIterable, Identifiable {
    case published = "published_at"
    case added = "created_at"
    case title
    case duration = "duration_secs"
    case position
    /// History only: play-recency order, local-only. A dedicated case so an
    /// EXPLICIT "Published desc" choice is distinguishable from the recency default.
    case recency

    var id: String { rawValue }

    var label: String {
        switch self {
        case .published: return "Published"
        case .added: return "Added"
        case .title: return "Title"
        case .duration: return "Duration"
        case .position: return "Position"
        case .recency: return "Recent"
        }
    }
}

/// One episode list's controls state: search + filter chips + order.
/// Persisted per page (the web's per-view-key sticky list config). Chips are
/// multi-select: OR within a facet, AND across facets (web query.rs).
struct ListQuery: Codable, Equatable {
    var search: String = ""
    var filters: Set<EpisodeFilter> = []
    var orderField: EpisodeOrderField = .published
    var direction: OrderDirection = .desc

    private enum CodingKeys: String, CodingKey {
        case search, filters, orderField, direction
    }

    init(
        search: String = "", filters: Set<EpisodeFilter> = [],
        orderField: EpisodeOrderField = .published, direction: OrderDirection = .desc
    ) {
        self.search = search
        self.filters = filters
        self.orderField = orderField
        self.direction = direction
    }

    /// Lenient decode: pre-multiselect snapshots (single `filter` key) just
    /// reset to no chips — never fail the whole restore over a vocabulary
    /// change.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        search = (try? c.decode(String.self, forKey: .search)) ?? ""
        filters = (try? c.decode(Set<EpisodeFilter>.self, forKey: .filters)) ?? []
        orderField = (try? c.decode(EpisodeOrderField.self, forKey: .orderField)) ?? .published
        direction = (try? c.decode(OrderDirection.self, forKey: .direction)) ?? .desc
    }

    /// Selected chips of each facet.
    var downloadChips: Set<EpisodeFilter> { filters.intersection(EpisodeFilter.downloadFacet) }
    var playedChips: Set<EpisodeFilter> { filters.intersection(EpisodeFilter.playedFacet) }

    /// Whether the server page must be re-filtered locally: a facet with
    /// MULTIPLE chips can't be expressed as a single wire token, so the fetch
    /// goes un-narrowed for that facet and `matchesChips` trims each page.
    var needsLocalChipFilter: Bool {
        downloadChips.count > 1 || playedChips.count > 1
    }

    /// Server-side query fragment (search + single-chip facets + order).
    /// Queue/History apply the same query locally instead.
    var queryItems: [URLQueryItem] {
        var items: [URLQueryItem] = []
        let trimmed = search.trimmingCharacters(in: .whitespaces)
        if !trimmed.isEmpty {
            items.append(URLQueryItem(name: "filter[search]", value: trimmed))
        }
        if downloadChips.count == 1, let token = downloadChips.first?.wireToken {
            items.append(URLQueryItem(name: "filter[download_status]", value: token))
        }
        if playedChips.count == 1, let token = playedChips.first?.wireToken {
            items.append(URLQueryItem(name: "filter[playback_status]", value: token))
        }
        if orderField != .position, orderField != .recency {
            items.append(URLQueryItem(name: "order[order_by]", value: orderField.rawValue))
            items.append(
                URLQueryItem(name: "order[direction]", value: direction == .asc ? "Asc" : "Desc"))
        }
        return items
    }

    /// Chip predicate over one row: OR within a facet, AND across facets (web
    /// apply_filter_sort). `isOnDevice` resolves the OnDevice chip (nil = no-op
    /// when callers switch to the device set wholesale); `status` should be the
    /// overlay read so a local mark-played flips results immediately.
    func matchesChips(
        _ episode: EpisodeData,
        isOnDevice: ((Int32) -> Bool)? = nil,
        status: ((EpisodeData) -> PlaybackStatus)? = nil
    ) -> Bool {
        if filters.contains(.onDevice), let isOnDevice, !isOnDevice(episode.id) {
            return false
        }
        let dl = downloadChips
        if !dl.isEmpty {
            let ok = dl.contains { chip in
                switch chip {
                case .downloaded: return episode.download_status == .downloaded
                case .downloading: return episode.download_status == .downloading
                default: return false
                }
            }
            if !ok { return false }
        }
        let played = playedChips
        if !played.isEmpty {
            let status = status?(episode) ?? episode.playback_status ?? .unplayed
            let ok = played.contains { chip in
                switch chip {
                case .unplayed: return status == .unplayed
                case .inProgress: return status == .played
                case .finished: return status == .finished
                default: return false
                }
            }
            if !ok { return false }
        }
        return true
    }

    /// Local projection of the same query (the queue's in-memory list).
    func apply(
        to episodes: [EpisodeData], isOnDevice: ((Int32) -> Bool)? = nil,
        status: ((EpisodeData) -> PlaybackStatus)? = nil
    ) -> [EpisodeData] {
        var out = episodes
        let trimmed = search.trimmingCharacters(in: .whitespaces).lowercased()
        if !trimmed.isEmpty {
            out = out.filter {
                $0.title.lowercased().contains(trimmed)
                    || ($0.description?.lowercased().contains(trimmed) ?? false)
                    || ($0.podcast?.title.lowercased().contains(trimmed) ?? false)
            }
        }
        out = out.filter { matchesChips($0, isOnDevice: isOnDevice, status: status) }
        switch orderField {
        case .recency:
            // History sorts recency itself (overlay-wins dates live there);
            // as a plain local sort this is a no-op passthrough.
            break
        case .position:
            // Base order IS position (asc) — desc just flips it (web parity;
            // previously direction was silently ignored for Position).
            if direction == .desc { out.reverse() }
        case .published:
            out.sort {
                direction == .asc
                    ? ($0.published_at ?? .distantPast) < ($1.published_at ?? .distantPast)
                    : ($0.published_at ?? .distantPast) > ($1.published_at ?? .distantPast)
            }
        case .added:
            out.sort {
                direction == .asc
                    ? $0.created_at < $1.created_at : $0.created_at > $1.created_at
            }
        case .title:
            out.sort {
                direction == .asc
                    ? $0.title.lowercased() < $1.title.lowercased()
                    : $0.title.lowercased() > $1.title.lowercased()
            }
        case .duration:
            out.sort {
                direction == .asc
                    ? ($0.duration_secs ?? 0) < ($1.duration_secs ?? 0)
                    : ($0.duration_secs ?? 0) > ($1.duration_secs ?? 0)
            }
        }
        return out
    }
}

/// The persistent sub-navbar on every episode list: search field + Filter
/// dropdown + Order dropdown (the web's list controls, menu-style).
struct ListControlsBar: View {
    @Binding var query: ListQuery
    /// The queue offers Position ordering; other pages don't.
    var allowsPosition = false
    /// History offers the Recent (play-recency) ordering; other pages don't.
    var allowsRecency = false
    /// Downloads fixes its facet via the segmented control — hide the filter
    /// menu there (every other list shows it, History included; web parity).
    var showFilter = true
    /// Embedded accounts have no separate device set — the OnDevice chip is
    /// dropped there (web: `embedded` prop on ListControls).
    var allowsOnDevice = true

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField("Search", text: $query.search)
                        .textFieldStyle(.plain)
                        .autocorrectionDisabled()
                    if !query.search.isEmpty {
                        Button {
                            query.search = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 9).fill(Color(.secondarySystemFill)))

                if showFilter {
                    filterMenu
                }

                Menu {
                    ForEach(orderFields) { field in
                        Button {
                            query.orderField = field
                        } label: {
                            if query.orderField == field {
                                Label(field.label, systemImage: "checkmark")
                            } else {
                                Text(field.label)
                            }
                        }
                    }
                    Divider()
                    Button {
                        query.direction = query.direction == .asc ? .desc : .asc
                    } label: {
                        Label(
                            query.direction == .asc ? "Ascending" : "Descending",
                            systemImage: query.direction == .asc ? "arrow.up" : "arrow.down")
                    }
                } label: {
                    Image(systemName: "arrow.up.arrow.down.circle")
                        .font(.title3)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            Divider()
        }
        .background(.bar)
    }

    private var orderFields: [EpisodeOrderField] {
        EpisodeOrderField.allCases.filter {
            (allowsPosition || $0 != .position) && (allowsRecency || $0 != .recency)
        }
    }

    /// Multi-select chips (web ListControls): tapping toggles membership;
    /// the menu stays coherent because facets OR internally / AND across.
    private var filterMenu: some View {
        Menu {
            ForEach(availableChips) { chip in
                Button {
                    if query.filters.contains(chip) {
                        query.filters.remove(chip)
                    } else {
                        query.filters.insert(chip)
                    }
                } label: {
                    if query.filters.contains(chip) {
                        Label(chip.label, systemImage: "checkmark")
                    } else {
                        Text(chip.label)
                    }
                }
            }
            if !query.filters.isEmpty {
                Divider()
                Button("Clear filters") { query.filters = [] }
            }
        } label: {
            Image(
                systemName: query.filters.isEmpty
                    ? "line.3.horizontal.decrease.circle"
                    : "line.3.horizontal.decrease.circle.fill"
            )
            .font(.title3)
        }
    }

    private var availableChips: [EpisodeFilter] {
        EpisodeFilter.allCases.filter { allowsOnDevice || $0 != .onDevice }
    }
}

/// The generic sort+search sub-navbar for NON-episode lists (podcasts,
/// playlists) — the web's shared `SortSearchControls` wrapper: a search field
/// plus a field/direction menu over a page-supplied field vocabulary.
struct SortSearchBar<Field: Hashable>: View {
    @Binding var search: String
    @Binding var field: Field
    @Binding var direction: OrderDirection
    /// Sort fields in menu order: (value, label).
    let fields: [(Field, String)]
    var placeholder = "Search"

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField(placeholder, text: $search)
                        .textFieldStyle(.plain)
                        .autocorrectionDisabled()
                    if !search.isEmpty {
                        Button {
                            search = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 9).fill(Color(.secondarySystemFill)))

                Menu {
                    ForEach(fields.indices, id: \.self) { idx in
                        Button {
                            field = fields[idx].0
                        } label: {
                            if field == fields[idx].0 {
                                Label(fields[idx].1, systemImage: "checkmark")
                            } else {
                                Text(fields[idx].1)
                            }
                        }
                    }
                    Divider()
                    Button {
                        direction = direction == .asc ? .desc : .asc
                    } label: {
                        Label(
                            direction == .asc ? "Ascending" : "Descending",
                            systemImage: direction == .asc ? "arrow.up" : "arrow.down")
                    }
                } label: {
                    Image(systemName: "arrow.up.arrow.down.circle")
                        .font(.title3)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            Divider()
        }
        .background(.bar)
    }
}
