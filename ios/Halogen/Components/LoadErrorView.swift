import SwiftUI

/// Full-screen load-failure state with a working Retry. Error branches render
/// this instead of a bare ContentUnavailableView: that branch has no scroll
/// container, so pull-to-refresh never works there — without the button the
/// only recovery was leaving the screen.
struct LoadErrorView: View {
    let title: String
    let message: String
    let retry: () async -> Void

    @State private var retrying = false

    var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: "wifi.exclamationmark")
        } description: {
            Text(message)
        } actions: {
            Button {
                guard !retrying else { return }
                retrying = true
                Task {
                    await retry()
                    retrying = false
                }
            } label: {
                if retrying {
                    ProgressView().controlSize(.small)
                } else {
                    Text("Retry")
                }
            }
            .buttonStyle(.borderedProminent)
            .accessibilityIdentifier("load-error-retry")
        }
    }
}
