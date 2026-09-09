use dioxus::prelude::*;

use halogen_webui_config::{ClientConfig, ClientConfigStore, ListViewStore, ListViews};
use halogen_webui_logging as logging;
use halogen_webui_platform::time::sleep_ms;

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

        // Load config and view state under the namespace set by AccountsProvider. Retry backend errors; persistent
        // failure permits default rendering but latches config saves off so defaults cannot erase stored auth. A fresh
        // boot retries recovery.
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
            halogen_webui_component_loading::LoadingSplash {}
        };
    }

    rsx! { {children} }
}
