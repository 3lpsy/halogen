import Foundation
import Observation

/// Server reachability for the navbar status dot: periodic `/healthz` probes
/// (polling stand-in for the web's connectivity-WS RTT; same three-state
/// surface). MainActor: the probe mutates @Observable state read by SwiftUI —
/// off-main mutation was a data race with body evaluation.
@MainActor
@Observable
final class ConnectionMonitor {
    enum Status: Equatable {
        case unknown
        case online
        case offline
    }

    private(set) var status: Status = .unknown
    /// User-forced offline (the navbar toggle, like the web's). Probes pause,
    /// the dot shows offline, and the outbox holds until back online.
    private(set) var manualOffline = false
    /// Fired whenever a probe finds the server reachable after it wasn't —
    /// the outbox drain trigger.
    var onOnline: (() -> Void)?
    private var baseUrl: String?
    private var probeTask: Task<Void, Never>?

    private static let interval: Duration = .seconds(15)

    /// (Re)start probing against a server. Called by the core once a session
    /// connects; safe to call again on account/server switch.
    func start(baseUrl: String) {
        self.baseUrl = baseUrl
        probeTask?.cancel()
        probeTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.probe()
                try? await Task.sleep(for: Self.interval)
            }
        }
    }

    func stop() {
        probeTask?.cancel()
        probeTask = nil
        status = .unknown
    }

    /// Flip the user-forced offline mode. Going online probes immediately
    /// (which also drains the outbox via onOnline).
    func setManualOffline(_ offline: Bool) {
        manualOffline = offline
        if offline {
            status = .offline
        } else {
            status = .unknown
            Task { await probe() }
        }
    }

    /// One immediate probe — the scene-active hook calls this so a return to
    /// the foreground doesn't wait out the interval.
    func probe() async {
        guard !manualOffline else { return }
        guard let baseUrl, let url = URL(string: "\(baseUrl)/healthz") else { return }
        var request = URLRequest(url: url)
        request.timeoutInterval = 5
        let previous = status
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            let ok = (response as? HTTPURLResponse).map { (200..<300).contains($0.statusCode) }
            status = ok == true ? .online : .offline
        } catch {
            status = .offline
        }
        if status == .online && previous != .online {
            onOnline?()
        }
    }
}
