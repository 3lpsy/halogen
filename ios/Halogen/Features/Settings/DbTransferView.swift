import SwiftUI
import UniformTypeIdentifiers

/// Database export/import (admin): a gzipped, scrubbed snapshot out via the
/// share sheet; merge an export back in via the file picker.
struct DbTransferView: View {
    let core: HalogenCore

    @State private var showImporter = false
    @State private var exportFile: URL?
    /// A picked file waits for an explicit confirm — appending a whole
    /// library shouldn't be one mis-tap away (web: staged_import).
    @State private var staged: (name: String, data: Data)?
    @State private var result: String?
    /// Embedded-only caveat after an import created users (see importDb).
    @State private var alignmentNote: String?
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        List {
            Section {
                Button {
                    Task { await exportDb() }
                } label: {
                    Label("Export database…", systemImage: "square.and.arrow.up")
                }
                .disabled(busy)
                if let exportFile {
                    ShareLink(item: exportFile) {
                        Label(exportFile.lastPathComponent, systemImage: "doc.badge.arrow.up")
                    }
                }
            } header: {
                Text("Export")
            } footer: {
                Text("A gzipped, scrubbed snapshot — no password hashes, download flags, or history.")
            }

            Section {
                if let staged {
                    Text(staged.name)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Button {
                        Task { await importDb(staged.data) }
                    } label: {
                        Label("Import now", systemImage: "square.and.arrow.down")
                    }
                    .disabled(busy)
                    Button("Cancel", role: .cancel) {
                        self.staged = nil
                    }
                    .disabled(busy)
                } else {
                    Button {
                        showImporter = true
                    } label: {
                        Label("Choose export file…", systemImage: "doc.badge.plus")
                    }
                    .disabled(busy)
                }
            } header: {
                Text("Import")
            } footer: {
                Text(
                    "Merges another server's export into this library: matching usernames merge, new users are created, nothing is replaced."
                )
            }

            if busy {
                HStack {
                    ProgressView()
                    Text("Working…").foregroundStyle(.secondary)
                }
            }
            if let result {
                Section("Result") { Text(result) }
            }
            if let alignmentNote {
                Section {
                    Text(alignmentNote).font(.footnote).foregroundStyle(.orange)
                }
            }
            if let error {
                Section { Text(error).font(.footnote).foregroundStyle(.red) }
            }
        }
        .navigationTitle("Database")
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(
            isPresented: $showImporter,
            allowedContentTypes: [.gzip, .data]
        ) { pick in
            switch pick {
            case .success(let url):
                stage(url)
            case .failure(let e):
                error = "\(e)"
            }
        }
    }

    /// Read the picked file immediately (the security scope is only valid
    /// now) and stage it for the explicit "Import now" confirm.
    private func stage(_ url: URL) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            staged = (url.lastPathComponent, try Data(contentsOf: url))
            error = nil
        } catch {
            self.error = "Could not read file: \(error)"
        }
    }

    private func exportDb() async {
        busy = true
        defer { busy = false }
        do {
            let (data, filename) = try await core.dbExport()
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(filename)
            try data.write(to: file)
            exportFile = file
            error = nil
        } catch {
            self.error = FriendlyError.message(error)
        }
    }

    private func importDb(_ data: Data) async {
        busy = true
        defer { busy = false }
        do {
            let summary = try await core.dbImport(data)
            // The web's composed success toast, verbatim shape.
            result =
                "Import merged: \(summary.users_merged) user(s) matched, "
                + "\(summary.users_created) created; +\(summary.podcasts_created) podcast(s), "
                + "+\(summary.episodes_created) episode(s), +\(summary.playlists_created) playlist(s)"
            // Users the import created got random passwords — rotate them
            // into the silent-login secrets so switching to them just works
            // (web: align_imported_users via recover_user). Warn only for
            // whatever couldn't be aligned.
            if core.isEmbeddedAccount, !summary.created_usernames.isEmpty {
                let failed = await core.alignImportedUsers(summary.created_usernames)
                alignmentNote =
                    failed.isEmpty
                    ? nil
                    : "Couldn't open imported profiles: \(failed.joined(separator: ", "))."
            } else {
                alignmentNote = nil
            }
            staged = nil
            error = nil
            await core.models?.podcasts.refresh()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
