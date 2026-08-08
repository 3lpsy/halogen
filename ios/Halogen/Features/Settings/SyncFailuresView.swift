import SwiftUI

/// Changes the server permanently rejected (dead-lettered sync ops). The
/// optimistic UI reverts on a later refresh; this page explains what and why.
struct SyncFailuresView: View {
    let failures: SyncFailures

    var body: some View {
        List {
            if failures.failures.isEmpty {
                ContentUnavailableView(
                    "No sync failures",
                    systemImage: "checkmark.circle",
                    description: Text("Changes the server rejects appear here."))
            } else {
                Section {
                    ForEach(failures.failures.reversed()) { failure in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(failure.summary)
                                .font(.callout.weight(.medium))
                            Text(failure.reason)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Text(failure.at, format: .relative(presentation: .named))
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                        .padding(.vertical, 2)
                    }
                } footer: {
                    Text("These changes were rejected by the server and undone. Redo one if you still want it.")
                }
            }
        }
        .navigationTitle("Sync failures")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            Button("Clear") { failures.clear() }
                .disabled(failures.failures.isEmpty)
        }
    }
}
