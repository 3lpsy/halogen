import SwiftUI

/// The infinite-scroll sentinel: a spinner that fires `loadMore` on appear,
/// or — after a failed page — a tappable retry. (A swallowed loadMore error
/// used to leave a spinner that never resolves at the fold.)
struct LoadMoreRow: View {
    let failed: Bool
    let loadMore: () async -> Void

    var body: some View {
        HStack {
            Spacer()
            if failed {
                Button {
                    Task { await loadMore() }
                } label: {
                    Label("Couldn't load more — retry", systemImage: "arrow.clockwise")
                        .font(.footnote)
                }
                .buttonStyle(.borderless)
            } else {
                ProgressView()
                    .onAppear { Task { await loadMore() } }
            }
            Spacer()
        }
        .listRowSeparator(.hidden)
    }
}
