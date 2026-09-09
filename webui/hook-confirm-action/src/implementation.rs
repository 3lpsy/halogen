//! Share confirmation open/busy state and configured-client error mapping. run returns Result<T, String> to the caller,
//! which chooses toast or inline feedback and supplies the actual server operation.

use dioxus::prelude::*;

use halogen_webui_config::ClientConfig;

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

    /// Run the confirmed action: flip `busy` on, build the API client from `config` (folding the canonical "No server
    /// configured." error into the same `String` channel as a request failure), `call` the server, then close the modal
    /// and hand the `Result<T, String>` to `on_result`. `busy` is cleared and the modal closed on every exit path;
    /// routing the result (toast vs inline-error signal) is the caller's, via `on_result`.
    pub fn run<T, Fut, Call, OnResult>(
        self,
        config: Signal<ClientConfig>,
        call: Call,
        on_result: OnResult,
    ) where
        T: 'static,
        Fut: std::future::Future<Output = Result<T, halogen_apiclient::ApiError>> + 'static,
        Call: FnOnce(halogen_apiclient::ApiClient) -> Fut + 'static,
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
