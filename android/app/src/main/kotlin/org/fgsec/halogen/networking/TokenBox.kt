package org.fgsec.halogen.networking

/// The JWT, shared by REFERENCE across client copies so an automatic refresh
/// (or login) propagates everywhere — including copies the core hands to
/// models. `onRefresh` lets the session store / art loader follow along.
class TokenBox(var token: String?) {
    var onRefresh: ((String) -> Unit)? = null

    /// Fired when a 401 survives the silent refresh — auth is dead (the web's
    /// `ToastDecision::SignOut` / worker `auth_expired` signal). The core
    /// reacts; the request still throws its 401 to the caller.
    var onAuthExpired: (() -> Unit)? = null
}
