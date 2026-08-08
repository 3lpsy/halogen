import SwiftUI
import UniformTypeIdentifiers

/// OPML import/export (admin — the web's Settings → Podcasts). Export shares
/// the server's OPML as a file; import reads a picked .opml/.xml and posts it.
struct OpmlView: View {
    let core: HalogenCore

    @State private var showImporter = false
    @State private var exportFile: URL?
    @State private var result: String?
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        List {
            Section("Export") {
                Button {
                    Task { await exportOpml() }
                } label: {
                    Label("Export subscriptions…", systemImage: "square.and.arrow.up")
                }
                .disabled(busy)
                if let exportFile {
                    ShareLink(item: exportFile) {
                        Label("Share halogen.opml", systemImage: "doc.badge.arrow.up")
                    }
                }
            }

            Section("Import") {
                Button {
                    showImporter = true
                } label: {
                    Label("Import OPML file…", systemImage: "square.and.arrow.down")
                }
                .disabled(busy)
            }

            if let result {
                Section("Result") {
                    Text(result)
                }
            }
            if let error {
                Section {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
        }
        .navigationTitle("OPML")
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(
            isPresented: $showImporter,
            allowedContentTypes: [.xml, UTType(filenameExtension: "opml") ?? .xml]
        ) { pick in
            switch pick {
            case .success(let url):
                Task { await importOpml(url) }
            case .failure(let e):
                error = "\(e)"
            }
        }
    }

    private func exportOpml() async {
        busy = true
        defer { busy = false }
        do {
            let opml = try await core.opmlExport()
            let dir = FileManager.default.temporaryDirectory
            let file = dir.appendingPathComponent("halogen.opml")
            try opml.write(to: file, atomically: true, encoding: .utf8)
            exportFile = file
            error = nil
        } catch {
            self.error = FriendlyError.message(error)
        }
    }

    private func importOpml(_ url: URL) async {
        busy = true
        defer { busy = false }
        do {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            let opml = try String(contentsOf: url, encoding: .utf8)
            let outcome = try await core.opmlImport(opml)
            result =
                "\(String(outcome.created)) created · \(String(outcome.skipped)) skipped · \(String(outcome.errors)) errors"
            await core.models?.podcasts.refresh()
            error = nil
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
