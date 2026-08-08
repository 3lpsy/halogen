import SwiftUI

/// The Playlists tab: every playlist (the queue badged as such), episode
/// counts from the EpisodeIds include, create/delete, drill into detail.
struct PlaylistsView: View {
    @Bindable var model: PlaylistsModel
    let core: HalogenCore

    @State private var showCreate = false
    @State private var renameTarget: PlaylistData?
    @State private var deleteTarget: PlaylistData?

    var body: some View {
        Group {
            if let error = model.error {
                LoadErrorView(title: "Couldn't load playlists", message: error) {
                    await model.refresh()
                }
            } else if model.loaded && model.playlists.isEmpty {
                ContentUnavailableView(
                    "No playlists yet",
                    systemImage: "music.note.list",
                    description: Text("Create a playlist to organize episodes.")
                )
            } else if model.loaded && model.displayed.isEmpty {
                ContentUnavailableView.search(text: model.query.search)
            } else {
                list
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            SortSearchBar(
                search: $model.query.search,
                field: $model.query.field,
                direction: $model.query.direction,
                fields: PlaylistSortField.allCases.map { ($0, $0.label) },
                placeholder: "Search playlists"
            )
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                EditButton()
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showCreate = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityIdentifier("playlist-add")
            }
        }
        .sheet(isPresented: $showCreate) {
            PlaylistCreateSheet(model: model, core: core)
        }
        .sheet(item: Binding(
            get: { renameTarget.map { RenameBox(playlist: $0) } },
            set: { renameTarget = $0?.playlist }
        )) { box in
            PlaylistEditSheet(model: model, core: core, playlist: box.playlist)
        }
        .task { await model.load() }
        .refreshable { await model.refresh() }
        .confirmationDialog(
            "Delete playlist?",
            isPresented: Binding(
                get: { deleteTarget != nil },
                set: { if !$0 { deleteTarget = nil } }
            ),
            titleVisibility: .visible
        ) {
            if let target = deleteTarget {
                Button("Delete \(target.name)", role: .destructive) {
                    Task { await model.delete(target) }
                    deleteTarget = nil
                }
            }
            Button("Cancel", role: .cancel) { deleteTarget = nil }
        } message: {
            Text("Removes the playlist everywhere. Its episodes stay in the library.")
        }
    }

    private var list: some View {
        List {
            ForEach(model.displayed, id: \.id) { playlist in
                HStack(spacing: 8) {
                    PlaylistRow(playlist: playlist)
                    Spacer(minLength: 0)
                    Menu {
                        PlaylistMenu(
                            playlist: playlist, model: model, core: core,
                            renameTarget: $renameTarget, deleteTarget: $deleteTarget)
                    } label: {
                        Image(systemName: "ellipsis")
                            .foregroundStyle(.secondary)
                            .frame(width: 32, height: 32)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderless)
                }
                .background(
                    NavigationLink(value: AppRoute.playlist(playlist.id)) { EmptyView() }
                        .opacity(0)
                )
                // Long-press mirror of the ellipsis menu (same shared content
                // — the two can't drift), like the episode rows.
                .contextMenu {
                    PlaylistMenu(
                        playlist: playlist, model: model, core: core,
                        renameTarget: $renameTarget, deleteTarget: $deleteTarget)
                }
                .swipeActions(edge: .trailing) {
                    Button(role: .destructive) {
                        deleteTarget = playlist
                    } label: {
                        Label("Delete", systemImage: "trash")
                    }
                }
            }
            .onMove { from, to in
                // Custom-asc + no search only (web rule) — offline-capable
                // optimistic reorder + a durable MovePlaylist op.
                guard model.reorderable else { return }
                model.move(fromOffsets: from, toOffset: to)
            }
        }
        .listStyle(.plain)
    }
}

struct PlaylistRow: View {
    let playlist: PlaylistData

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: playlist.is_default ? "list.bullet.circle.fill" : "music.note.list")
                .font(.title2)
                .foregroundStyle(playlist.is_default ? Color.accentColor : .secondary)
                .frame(width: 36)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(playlist.name).font(.headline)
                    if playlist.is_default {
                        Text("Queue")
                            .font(.caption2.weight(.semibold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(Color.accentColor.opacity(0.15)))
                            .foregroundStyle(Color.accentColor)
                    }
                }
                if let count = playlist.episode_ids?.count {
                    Text("\(count) episodes")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct PlaylistCreateSheet: View {
    let model: PlaylistsModel
    let core: HalogenCore
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var description = ""
    @State private var makeDefault = false
    @State private var deleteServerFile = false
    @State private var deleteClientFile = false
    @State private var error: String?

    /// No queue yet → the first playlist MUST become it (web: force_default;
    /// the server enforces this too).
    private var forceDefault: Bool {
        core.models?.queue.loaded == true && core.models?.queue.queue == nil
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                    TextField("Description (optional)", text: $description, axis: .vertical)
                }
                Section {
                    Toggle("Make this the queue (default)", isOn: $makeDefault)
                        .disabled(forceDefault)
                } footer: {
                    if forceDefault {
                        Text("You don't have a queue yet — this playlist will become it.")
                    }
                }
                Section {
                    Toggle("Delete server download on remove", isOn: $deleteServerFile)
                    Toggle("Delete device download on remove", isOn: $deleteClientFile)
                } footer: {
                    Text(
                        "Removing an episode from this playlist also deletes its downloaded file on the server (unless another playlist still has it) and/or on the removing device."
                    )
                }
                if let error {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
            .navigationTitle("New Playlist")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear {
                if forceDefault { makeDefault = true }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") {
                        Task {
                            do {
                                let desc = description.trimmingCharacters(in: .whitespaces)
                                try await model.create(
                                    name: name.trimmingCharacters(in: .whitespaces),
                                    description: desc.isEmpty ? nil : desc,
                                    isDefault: makeDefault || forceDefault,
                                    deleteServerFile: deleteServerFile,
                                    deleteClientFile: deleteClientFile)
                                dismiss()
                            } catch {
                                self.error = FriendlyError.message(error)
                            }
                        }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}


/// Identifiable wrapper (generated DTOs aren't Identifiable).
private struct RenameBox: Identifiable {
    let playlist: PlaylistData
    var id: Int32 { playlist.id }
}

/// Per-playlist quick actions: edit, smart reorder, make-queue, delete.
struct PlaylistMenu: View {
    let playlist: PlaylistData
    let model: PlaylistsModel
    let core: HalogenCore
    @Binding var renameTarget: PlaylistData?
    @Binding var deleteTarget: PlaylistData?

    var body: some View {
        Button {
            renameTarget = playlist
        } label: {
            Label("Edit", systemImage: "pencil")
        }

        Menu {
            reorderButtons(direction: .asc, label: "Ascending")
            reorderButtons(direction: .desc, label: "Descending")
        } label: {
            Label("Reorder by", systemImage: "arrow.up.arrow.down")
        }

        if !playlist.is_default {
            Button {
                Task {
                    if core.isOffline {
                        // Offline: optimistic flip + queued UpdatePlaylist
                        // (web rule — edits queue offline, go direct online).
                        model.markDefaultLocally(id: playlist.id)
                        await core.outbox?.enqueue(
                            .updatePlaylist(
                                playlistId: playlist.id, name: nil, isDefault: true,
                                description: nil, deleteServerFile: nil,
                                deleteClientFile: nil))
                    } else {
                        do {
                            try await core.makeQueuePlaylist(id: playlist.id)
                        } catch {
                            ToastCenter.shared.error(
                                "Couldn't make \"\(playlist.name)\" the queue — \(FriendlyError.message(error))"
                            )
                        }
                        await model.refresh()
                    }
                }
            } label: {
                Label("Make this the queue", systemImage: "list.bullet.circle")
            }
        }

        Divider()
        Button(role: .destructive) {
            // Deleting is irreversible (server rows + files) — confirm first.
            deleteTarget = playlist
        } label: {
            Label("Delete", systemImage: "trash")
        }
    }

    @ViewBuilder
    private func reorderButtons(direction: OrderDirection, label: String) -> some View {
        Menu(label) {
            ForEach(PlaylistReorderField.allCases, id: \.self) { field in
                Button(String(describing: field).capitalized) {
                    Task {
                        // Durable (web: ReorderPlaylist op) — queues offline
                        // instead of silently dropping the action.
                        await core.outbox?.enqueue(
                            .reorderPlaylist(
                                playlistId: playlist.id, field: field, direction: direction))
                        await model.refresh()
                    }
                }
            }
        }
    }
}

/// Edit a playlist (`PUT /playlists/{id}`) — the web form's full field set:
/// name, description, make-default, and the two delete-on-remove flags.
struct PlaylistEditSheet: View {
    let model: PlaylistsModel
    let core: HalogenCore
    let playlist: PlaylistData

    @Environment(\.dismiss) private var dismiss
    @State private var name: String = ""
    @State private var description: String = ""
    @State private var makeDefault = false
    @State private var deleteServerFile = false
    @State private var deleteClientFile = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                    TextField("Description (optional)", text: $description, axis: .vertical)
                }
                Section {
                    Toggle("Make this the queue (default)", isOn: $makeDefault)
                        .disabled(playlist.is_default)
                }
                Section {
                    Toggle("Delete server download on remove", isOn: $deleteServerFile)
                    Toggle("Delete device download on remove", isOn: $deleteClientFile)
                } footer: {
                    Text(
                        "Removing an episode from this playlist also deletes its downloaded file on the server (unless another playlist still has it) and/or on the removing device."
                    )
                }
                if let error {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
            .navigationTitle("Edit playlist")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task { await save() }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear {
                name = playlist.name
                description = playlist.description ?? ""
                makeDefault = playlist.is_default
                deleteServerFile = playlist.on_remove_delete_file_server ?? false
                deleteClientFile = playlist.on_remove_delete_file_client ?? false
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func save() async {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        let desc = description.trimmingCharacters(in: .whitespaces)
        // The web edit sends the FULL prefilled set, so unchanged fields
        // round-trip their current values (nil would mean "leave unchanged"
        // and make blanked fields unreachable).
        if core.isOffline {
            // Offline: optimistic + queued UpdatePlaylist (online goes
            // direct so the form can show server errors — web rule).
            model.updateLocally(
                id: playlist.id, name: trimmed, description: desc.isEmpty ? nil : desc,
                isDefault: makeDefault, deleteServerFile: deleteServerFile,
                deleteClientFile: deleteClientFile)
            await core.outbox?.enqueue(
                .updatePlaylist(
                    playlistId: playlist.id, name: trimmed, isDefault: makeDefault,
                    description: desc.isEmpty ? nil : desc,
                    deleteServerFile: deleteServerFile,
                    deleteClientFile: deleteClientFile))
            dismiss()
            return
        }
        do {
            try await core.updatePlaylist(
                id: playlist.id, name: trimmed,
                description: desc.isEmpty ? nil : desc,
                isDefault: makeDefault,
                deleteServerFile: deleteServerFile,
                deleteClientFile: deleteClientFile)
            await model.refresh()
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
