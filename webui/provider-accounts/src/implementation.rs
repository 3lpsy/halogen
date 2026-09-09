use dioxus::prelude::*;

use halogen_webui_accounts::accounts::{Accounts, AccountsStore};
use halogen_webui_platform::namespace;

/// Keep the account registry above the keyed user subtree. Switching remounts config, worker, stores, and player under
/// a freshly set namespace, stopping old audio and rerunning normal hydration/auth. Set the ambient namespace
/// synchronously before children mount.
#[component]
pub fn AccountsProvider(children: Element) -> Element {
    let accounts = use_signal(Accounts::default);
    let ready = use_signal(|| false);

    use_future(move || async move {
        let loaded = AccountsStore::load().await;
        let mut accounts = accounts;
        let mut ready = ready;
        halogen_webui_logging::info!(
            users = loaded.users.len(),
            active = ?loaded.active_user_id,
            "Account registry loaded"
        );
        accounts.set(loaded);
        ready.set(true);
    });

    use_context_provider(|| accounts);

    // Mirror the active account into the ambient namespace on every render, before the keyed subtree below
    // mounts/remounts (its stores read the namespace as they open). Reading the signal here also subscribes us, so a
    // switch re-renders this provider → updates the namespace → re-keys the subtree. The kind rides along: embedded
    // accounts live under `e{id}`.
    let active = accounts.read().active_key();
    namespace::set_active(
        active.map(|k| k.id),
        active.is_some_and(|k| k.is_embedded()),
        active.map(|k| k.server).unwrap_or(0),
    );

    if !ready() {
        return rsx! {
            halogen_webui_component_loading::LoadingSplash {}
        };
    }

    // Emit the user-keyed wrapper through a one-item iterator: Dioxus remounts keyed list children, not same-template
    // root nodes. display:contents avoids layout impact; take moves the non-Clone children.
    let subtree_key = namespace::segment_for(
        active.map(|k| k.id),
        active.is_some_and(|k| k.is_embedded()),
        active.map(|k| k.server).unwrap_or(0),
    );
    let mut children_once = Some(children);
    rsx! {
        {std::iter::once(()).map(move |_| {
            let ch = children_once.take().expect("subtree rendered once");
            rsx! {
                div { key: "{subtree_key}", class: "contents", {ch} }
            }
        })}
    }
}
