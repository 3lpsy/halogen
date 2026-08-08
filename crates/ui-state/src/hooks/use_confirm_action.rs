//! Shared "confirm, then run one server action" scaffolding for the config pages
//! (`pages/view_config`'s restart, `pages/config_overrides_form`'s save + clear).
//!
//! Each of those flows wraps a `ConfirmModal` (in `halogen-ui-widgets`) in
//! the same envelope: a `busy` flag that flips true→false around an `await`, an
//! `open` flag the modal reads, and an `await` that builds the API client via
//! [`ClientConfig::api_client_or_err`], runs one call, and folds its
//! [`ApiError`](halogen_api::ApiError) into a `String`. What stays per-site is the
//! call itself and how the `Result` is routed (a toast vs an inline-error signal),
//! so [`ConfirmAction::run`] hands the `Result<T, String>` back to an `on_result`
//! closure rather than deciding for the caller.

use dioxus::prelude::*;

use halogen_ui_config::ClientConfig;

/// The two signals every confirm-action flow carries: `open` (does the modal
/// show?) and `busy` (is the action in flight?). `Copy` (both fields are
/// `Signal`s) so it can be captured by the modal's callbacks.
#[derive(Clone, Copy)]
pub struct ConfirmAction {
    /// Whether the confirmation modal is currently rendered.
    pub open: Signal<bool>,
    /// True while the confirmed action's request is in flight — drives the
    /// modal's spinner/disabled state and the trigger button's `disabled`.
    pub busy: Signal<bool>,
}

/// Create a [`ConfirmAction`] signal pair (`open=false`, `busy=false`).
pub fn use_confirm_action() -> ConfirmAction {
    ConfirmAction {
        open: use_signal(|| false),
        busy: use_signal(|| false),
    }
}

impl ConfirmAction {
    /// Open the confirmation modal.
    pub fn open(mut self) {
        self.open.set(true);
    }

    /// Close the confirmation modal.
    pub fn close(mut self) {
        self.open.set(false);
    }

    /// Run the confirmed action: flip `busy` on, build the API client from
    /// `config` (folding the canonical "No server configured." error into the
    /// same `String` channel as a request failure), `call` the server, then close
    /// the modal and hand the `Result<T, String>` to `on_result`. `busy` is
    /// cleared and the modal closed on every exit path; routing the result (toast
    /// vs inline-error signal) is the caller's, via `on_result`.
    pub fn run<T, Fut, Call, OnResult>(
        self,
        config: Signal<ClientConfig>,
        call: Call,
        on_result: OnResult,
    ) where
        T: 'static,
        Fut: std::future::Future<Output = Result<T, halogen_api::ApiError>> + 'static,
        Call: FnOnce(halogen_api::ApiClient) -> Fut + 'static,
        OnResult: FnOnce(Result<T, String>) + 'static,
    {
        let mut open = self.open;
        let mut busy = self.busy;
        busy.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            let res = match cfg.api_client_or_err() {
                Ok(client) => call(client).await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            busy.set(false);
            open.set(false);
            on_result(res);
        });
    }
}
