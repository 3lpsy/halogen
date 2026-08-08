//! Shared reactive-form scaffolding for the create/edit form pages
//! (`pages/playlist_form`, `podcast_config_form`, `user_edit`, `podcast_auto_playlists`).
//!
//! These forms all carry the same async/error trio of signals and the same
//! "no server configured" fallback they set when the submit path can't build an
//! API client. Both are factored out here; the per-field inline-error merge is
//! shared via [`FormErrors::field_messages`], and the identical online-submit
//! envelope (toggle `submitting`, build the client, fold an `Err` into
//! `server_errors`) is shared via [`FormState::spawn_submit`]. What stays per-form
//! is what genuinely differs: local validation, the offline-edit branch, the API
//! call itself, and the success handler.

use dioxus::prelude::*;
use halogen_api::{ApiClient, ApiError};

use halogen_ui_config::ClientConfig;
use halogen_ui_forms::FormErrors;

/// The canonical "no server configured" form error — the fallback a submit path
/// sets when it can't build an API client. Mirrors the message
/// `ClientConfig::api_client_or_err` uses, routed through
/// [`FormErrors::from_api_error`] so it lands in the catch-all banner exactly as
/// the hand-rolled version did.
pub fn no_server_configured_error() -> FormErrors {
    FormErrors::from_api_error(&ApiError::Server {
        status: 0,
        message: "No server configured.".into(),
    })
}

/// The async + error signals every form page carries: an in-flight `submitting`
/// flag, the latest `server_errors` (`None` until a submit fails), and a
/// `submitted` flag that ungates inline field errors after the first attempt.
///
/// `Copy` (all fields are `Signal`s) so it can be captured by the submit closure.
#[derive(Clone, Copy)]
pub struct FormState {
    /// True while a submit request is in flight (disables the button + shows a spinner).
    pub submitting: Signal<bool>,
    /// Server-side errors from the last failed submit, folded into the form's
    /// [`FormErrors`] surface. `None` before any failure / once cleared on edit.
    pub server_errors: Signal<Option<FormErrors>>,
    /// Set on the first submit attempt — ungates "required"/local field errors so an
    /// untouched-but-empty field is flagged once the user tries to submit.
    pub submitted: Signal<bool>,
}

/// Create the [`FormState`] signal trio (`submitting=false`, `server_errors=None`,
/// `submitted=false`).
pub fn use_form_state() -> FormState {
    FormState {
        submitting: use_signal(|| false),
        server_errors: use_signal(|| Option::<FormErrors>::None),
        submitted: use_signal(|| false),
    }
}

impl FormState {
    /// Run the standard online submit: flip `submitting` on, build the API client
    /// from `config` (setting the canonical "no server configured" error and
    /// bailing if it's absent), `call` the server, then either hand the value to
    /// `on_ok` or fold the failure into `server_errors`. `submitting` is cleared on
    /// every exit path. The caller still owns what differs — local validation, the
    /// offline-edit branch, the API call inside `call`, and the success handler.
    ///
    /// `on_ok` returns a future so a success handler can `await` (e.g. persist the
    /// device account registry); a purely synchronous handler just wraps its body
    /// in `async move { … }`.
    pub fn spawn_submit<T, Fut, Call, OnOk, OkFut>(
        self,
        config: Signal<ClientConfig>,
        call: Call,
        on_ok: OnOk,
    ) where
        T: 'static,
        Fut: std::future::Future<Output = Result<T, ApiError>> + 'static,
        Call: FnOnce(ApiClient) -> Fut + 'static,
        OnOk: FnOnce(T) -> OkFut + 'static,
        OkFut: std::future::Future<Output = ()> + 'static,
    {
        let mut submitting = self.submitting;
        let mut server_errors = self.server_errors;
        submitting.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            let Some(client) = cfg.api_client() else {
                submitting.set(false);
                server_errors.set(Some(no_server_configured_error()));
                return;
            };
            let result = call(client).await;
            submitting.set(false);
            match result {
                Ok(value) => on_ok(value).await,
                Err(e) => server_errors.set(Some(FormErrors::from_api_error(&e))),
            }
        });
    }
}
