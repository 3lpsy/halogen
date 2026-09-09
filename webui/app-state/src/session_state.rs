//! Session/auth-liveness state, the worker-owned reactive slice. Tiny by design: it carries only `auth_expired`, the
//! worker-owned bit that the credentials (`server_url`/`access_token`) themselves do NOT, those live in `ClientConfig`
//! (the persisted source of truth). Its own signal so flipping it re-renders only the `WorkerProvider` effect that
//! watches it (clear token → sign out), not every `EpisodeState` subscriber. The sync worker is the only writer.

/// Worker-owned session liveness. Set when an authed request returns 401; the
/// `WorkerProvider` watches it to clear the stored token and sign out, after which
/// `RootGuard` redirects to login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    /// `true` once the worker has seen a 401 (token dead). Stays true until the
    /// store re-mounts under a fresh login.
    pub auth_expired: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_live() {
        assert!(!SessionState::default().auth_expired);
    }
}
