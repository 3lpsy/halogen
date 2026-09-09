/// Serializes journal/cache transactions across handles for the same account.
actor JournalGate {
    private actor Registry {
        var gates: [String: JournalGate] = [:]
        func gate(_ path: String) -> JournalGate {
            if let gate = gates[path] { return gate }
            let gate = JournalGate(); gates[path] = gate; return gate
        }
    }
    private static let registry = Registry()
    static func forPath(_ path: String) async -> JournalGate { await registry.gate(path) }

    private var busy = false
    private var waiting: [CheckedContinuation<Void, Never>] = []
    func acquire() async {
        if !busy { busy = true; return }
        await withCheckedContinuation { waiting.append($0) }
    }
    func release() {
        if waiting.isEmpty { busy = false } else { waiting.removeFirst().resume() }
    }
}
