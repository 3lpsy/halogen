//! Folding both legs of a cycle into the reachability the UI shows.

/// What one leg of a cycle (push or pull) concluded about the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Completed — or had nothing to send, which on its own proves nothing.
    Ok,
    /// Transport failure: no connectivity.
    Offline,
    /// Credentials rejected beyond the client's own transparent refresh.
    AuthRejected,
    /// The server answered but the leg did not complete (5xx, decode, apply).
    Failed,
}

/// Reachability published to the status surface after a cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleStatus {
    /// The cycle reached the server.
    pub online: bool,
    /// Credentials are dead — paused until re-login.
    pub auth_paused: bool,
}

/// Fold both legs; `pull` is `None` when the pull never ran. Push short-circuits with **no network I/O** on an
/// empty outbox, so for an idle device the pull is the only leg that can prove reachability at all (CORE-038 /
/// WEB-020 / IOS-016). A leg that reached the server but failed leaves `online` true — "reachable but failing"
/// is a distinct state, carried by `last_error`.
pub fn cycle_status(push: Reach, pull: Option<Reach>) -> CycleStatus {
    let mut status = CycleStatus {
        online: true,
        auth_paused: false,
    };
    for leg in [Some(push), pull].into_iter().flatten() {
        match leg {
            Reach::Offline => status.online = false,
            Reach::AuthRejected => {
                status.online = false;
                status.auth_paused = true;
            }
            Reach::Ok | Reach::Failed => {}
        }
    }
    status
}

/// Classify a non-2xx pull response. A 401 here already survived the client's
/// transparent refresh, so the refresh token is dead.
pub fn reach_of_status(status: u16) -> Reach {
    if status == 401 {
        Reach::AuthRejected
    } else {
        Reach::Failed
    }
}

/// Classify a request that never produced a response.
pub fn reach_of_transport(is_offline: bool) -> Reach {
    if is_offline {
        Reach::Offline
    } else {
        Reach::Failed
    }
}
