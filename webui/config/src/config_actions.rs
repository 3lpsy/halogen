//! Write actions over the app-wide [`ClientConfig`] signal. The read side (`use_config`, a hook in halogen-webui-state)
//! reads the signal; the mutate-and-persist *action* lives here alongside the other service actions `account_actions`
//! (in halogen-webui-accounts) rather than in a `use_*` file, since it's a plain function, not a hook.

use dioxus::prelude::*;

use crate::{ClientConfig, ClientConfigStore};

/// Apply `mutate` to the live config and persist the result (snapshot + spawn a
/// save to the active user's namespace). The common "edit a setting and save it"
/// action — `peek`s the snapshot (not a reactive read) so the save future doesn't
/// resubscribe the caller.
pub fn persist_config(mut config: Signal<ClientConfig>, mutate: impl FnOnce(&mut ClientConfig)) {
    mutate(&mut config.write());
    let snapshot = config.peek().clone();
    spawn(async move {
        ClientConfigStore::save(&snapshot).await;
    });
}
