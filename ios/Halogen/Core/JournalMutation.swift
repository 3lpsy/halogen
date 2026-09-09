import Foundation

extension HalogenCore {
    func forAccount(_ originStore: LocalStore?) throws -> HalogenCore {
        guard store === originStore else { throw CancellationError() }
        return self
    }

    /// Persist intent before deriving UI state, and ignore completions from a prior account.
    func ensureQueued(originStore: LocalStore? = nil, _ kind: OutboxOp.Kind) async -> Bool {
        await ensureQueuedBatch(originStore: originStore, [kind])
    }

    func ensureQueuedBatch(originStore: LocalStore? = nil, _ kinds: [OutboxOp.Kind]) async -> Bool {
        if let originStore, store !== originStore { return false }
        guard let queue = outbox else {
            ToastCenter.shared.error("Sync storage is unavailable. This change wasn't saved.")
            return false
        }
        let saved = await queue.enqueueBatch(kinds)
        return saved && outbox === queue
    }

    func enqueueMutation(
        originStore: LocalStore? = nil, _ kind: OutboxOp.Kind, episode: EpisodeData? = nil,
        update: @escaping @MainActor () -> Void
    ) {
        if let originStore, store !== originStore { return }
        let origin = outbox
        let cache = store
        Task { @MainActor in
            guard let origin, outbox === origin else { return }
            if let episode, let cache {
                do { try await cache.saveDurably(episode, key: CacheKey.episode(episode.id)) } catch {
                    ToastCenter.shared.error("Couldn't save episode data. Please try again.")
                    return
                }
            }
            guard outbox === origin, await ensureQueued(kind) else { return }
            update()
        }
    }
}
