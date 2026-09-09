import Foundation
import Observation

/// Session-owned invalidation for committed sync snapshots and completed feed polls.
@MainActor
@Observable
final class LibraryChanges {
    private(set) var revision = 0
    private var generation = UUID()
    private var polls: [UInt64: Task<Void, Never>] = [:]

    func invalidate() { revision += 1 }

    func cancel() {
        generation = UUID()
        for task in polls.values { task.cancel() }
        polls.removeAll()
    }

    /// Poll completion is independent of the initiating screen's lifetime.
    @discardableResult
    func watch(
        jobId: UInt64,
        status: @escaping @MainActor () async throws -> PollJobStatus,
        pause: @escaping @MainActor () async throws -> Void = { try await Task.sleep(for: .seconds(1)) }
    ) -> Task<Void, Never> {
        if let existing = polls[jobId] { return existing }
        let origin = generation
        let task = Task { [weak self] in
            for _ in 0..<300 {
                guard !Task.isCancelled, self?.generation == origin else { break }
                do {
                    let result = try await status()
                    guard !Task.isCancelled, self?.generation == origin else { break }
                    if result != .running {
                        // Failed jobs can still have committed episodes from other feeds.
                        self?.invalidate()
                        break
                    }
                } catch is CancellationError {
                    break
                } catch {
                    // A transient transport failure must not lose the completion watch.
                }
                do { try await pause() } catch { break }
            }
            if self?.generation == origin { self?.polls[jobId] = nil }
        }
        polls[jobId] = task
        return task
    }
}
