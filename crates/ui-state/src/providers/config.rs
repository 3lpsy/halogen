use dioxus::prelude::*;

use halogen_ui_config::{ClientConfig, ClientConfigStore, ListViewStore, ListViews};
use halogen_ui_logging as logging;
use halogen_ui_platform::time::sleep_ms;

/// How often newly captured device-log lines are flushed to persistent storage.
const LOG_FLUSH_MS: u32 = 2000;

/// Loads the persisted [`ClientConfig`] and provides it as `Signal<ClientConfig>`.
///
/// Renders a splash until the (async, on native) load completes so downstream
/// guards never see a transient default config.
#[component]
pub fn ConfigProvider(children: Element) -> Element {
    let config = use_signal(ClientConfig::default);
    // Remembered per-list view state, kept OUT of the auth-bearing `config` signal
    // so view-state churn (every settled sort/filter change) re-renders nobody and
    // never reaches the sync worker. Only `use_list_view_state` touches it, via
    // peek/write — there are no reactive readers, so its writes notify no one.
    let list_views = use_signal(ListViews::default);
    let ready = use_signal(|| false);

    use_future(move || async move {
        // Replay persisted device logs into the in-app ring as early as possible,
        // then apply the saved capture settings to the live logger so the gate
        // matches the user's choice (the profile default applied at `init()`
        // until now).
        logging::ingest_persisted(logging::store::load_all().await);

        // Load the active user's config + view state. `AccountsProvider` sets the
        // ambient namespace before this subtree mounts, so both stores read the
        // right user (or the `anon` namespace pre-login).
        //
        // The config load is retried on backend FAILURE (not absence): a
        // transient IndexedDB fault here would fake a signed-out default —
        // and a save of that default permanently overwrites the real config,
        // auth token included. If it still fails after the retries we proceed
        // with defaults, but `ClientConfigStore` has latched its degraded
        // state, so every save is refused for the session (signed-out-looking
        // but harmless; a reload retries from scratch).
        const LOAD_RETRIES: u32 = 4;
        const LOAD_RETRY_DELAY_MS: u32 = 2_000;
        let mut attempt = 0;
        let loaded = loop {
            match ClientConfigStore::try_load().await {
                Ok(config) => break config,
                Err(e) if attempt < LOAD_RETRIES => {
                    attempt += 1;
                    logging::warn!(
                        error = %e,
                        attempt,
                        "Client config load failed; retrying"
                    );
                    sleep_ms(LOAD_RETRY_DELAY_MS).await;
                }
                Err(e) => {
                    logging::error!(
                        error = %e,
                        "Client config load failed after retries — running degraded \
                         (defaults shown, config saves disabled)"
                    );
                    break ClientConfig::default();
                }
            }
        };
        let loaded_views = ListViewStore::load().await;
        logging::set_enabled(loaded.device_logs.enabled);
        logging::set_level(loaded.device_logs.level);
        logging::info!(
            authenticated = loaded.is_authenticated(),
            server_configured = loaded.server_url.is_some(),
            "Client config loaded"
        );

        let mut config = config;
        let mut list_views = list_views;
        let mut ready = ready;
        config.set(loaded);
        list_views.set(loaded_views);
        ready.set(true);
    });

    // Flush newly captured device-log lines to persistent storage on an interval.
    // Lives here (always mounted, runs on both targets) rather than at a call
    // site so capture itself stays synchronous and cheap.
    use_future(|| async move {
        loop {
            sleep_ms(LOG_FLUSH_MS).await;
            let pending = logging::drain_pending();
            if !pending.is_empty() {
                logging::store::append(&pending).await;
            }
        }
    });

    use_context_provider(|| config);
    use_context_provider(|| list_views);

    if !ready() {
        return rsx! {
            super::LoadingSplash {}
        };
    }

    rsx! { {children} }
}
