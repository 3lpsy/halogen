import SwiftUI

/// Polling history (admin): recent poll jobs with per-run totals, and a
/// "poll now" trigger. Online-only, like the web page.
struct PollingView: View {
    let core: HalogenCore

    @State private var jobs: [PollJobData] = []
    @State private var error: String?
    @State private var polling = false

    var body: some View {
        Group {
            if let error {
                ContentUnavailableView {
                    Label("Couldn't load poll jobs", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error).font(.footnote.monospaced())
                }
            } else if jobs.isEmpty {
                ContentUnavailableView(
                    "No poll jobs yet",
                    systemImage: "arrow.triangle.2.circlepath",
                    description: Text("Trigger a poll to fetch new episodes.")
                )
            } else {
                List(jobs, id: \.id) { job in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            statusBadge(job.status)
                            Text(job.started_at, format: .dateTime.month().day().hour().minute())
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text("#\(String(job.id))")
                                .font(.caption2.monospaced())
                                .foregroundStyle(.tertiary)
                        }
                        Text("\(String(job.total_new)) new · \(String(job.total_updated)) updated · \(String(job.total_errors)) errors")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 2)
                }
                .listStyle(.plain)
            }
        }
        .halogenNavbar(core: core)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    Task { await pollNow() }
                } label: {
                    if polling {
                        ProgressView()
                    } else {
                        Image(systemName: "arrow.clockwise")
                    }
                }
                .accessibilityIdentifier("poll-now")
                .disabled(polling)
            }
        }
        .task { await load() }
        .refreshable { await load() }
    }

    @ViewBuilder
    private func statusBadge(_ status: PollJobStatus) -> some View {
        Text(String(describing: status).capitalized)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Capsule().fill(badgeColor(status).opacity(0.15)))
            .foregroundStyle(badgeColor(status))
    }

    private func badgeColor(_ status: PollJobStatus) -> Color {
        switch status {
        case .completed: return .green
        case .running: return .blue
        case .failed: return .red
        }
    }

    private func load() async {
        do {
            jobs = try await core.pollJobs()
            error = nil
        } catch {
            if jobs.isEmpty { self.error = FriendlyError.message(error) }
        }
    }

    private func pollNow() async {
        polling = true
        defer { polling = false }
        do {
            _ = try await core.startPollJob()
            ToastCenter.shared.success("Poll started")
        } catch {
            // A silent failure here left no feedback at all (web surfaces
            // every non-form mutation outcome through the toast queue).
            ToastCenter.shared.error("Couldn't start poll: \(error)")
        }
        await load()
    }
}
