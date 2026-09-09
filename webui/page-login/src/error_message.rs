//! Shared auth error mapping: `AuthErrorContext` + `auth_error_message` — one
//! funnel for the login page's inline error string (kept per-context so the
//! reachability probe and the login attempt keep their own wording).

use halogen_apiclient::ApiError;

/// Which STEP of the connect flow is mapping the error: the pre-login health
/// probe (`Setup`) never sees a 401; the login attempt does.
#[derive(Clone, Copy)]
pub(crate) enum AuthErrorContext {
    /// The login attempt itself.
    Login,
    /// The reachability (health) probe that precedes it.
    Setup,
}

/// Map an [`ApiError`] to the single inline error string the auth card renders. The page keeps one `Option<String>`
/// error signal (not the form-wide `FormErrors` map). Wording stays per-context because the steps differ: login shows
/// "Invalid credentials" for a 401 and prefixes unknown errors with "Login failed:", while the health probe's transport
/// message nudges the user to check the server is running.
pub(crate) fn auth_error_message(err: &ApiError, ctx: AuthErrorContext) -> String {
    match (ctx, err) {
        // Login's invalid-credentials path — setup never authenticates so it has
        // no 401 case and falls through to the generic server handling below.
        (AuthErrorContext::Login, ApiError::Server { status: 401, .. }) => {
            "Invalid credentials".into()
        }
        (AuthErrorContext::Login, ApiError::Transport(_)) => "Server unreachable".into(),
        (AuthErrorContext::Login, e) => format!("Login failed: {e}"),

        (AuthErrorContext::Setup, ApiError::Transport(_)) => {
            "Server unreachable. Is it running?".into()
        }
        (AuthErrorContext::Setup, ApiError::Server { status, message }) => {
            format!("Server error {status}: {message}")
        }
        (AuthErrorContext::Setup, ApiError::Decode(msg)) => {
            format!("Could not reach server: {msg}")
        }
        (AuthErrorContext::Setup, _) => "Unexpected error".into(),
    }
}
