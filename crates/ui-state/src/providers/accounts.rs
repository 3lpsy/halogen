use dioxus::prelude::*;

use halogen_ui_accounts::accounts::{Accounts, AccountsStore};
use halogen_ui_platform::namespace;

/// Loads the device-global account registry and provides it as
/// `Signal<Accounts>`, then renders the per-user data subtree **keyed on the
/// active user id**.
///
/// This provider sits ABOVE the per-user providers (config / state / worker /
/// player) and never remounts — it holds the cross-account state (the list of
/// users and who's active). The keyed wrapper is the hot-swap mechanism: changing
/// `active_user_id` changes the key, so Dioxus tears down the
/// whole data subtree (the worker coroutine ends, the local/media store handles
/// drop, the player controller drops → audio stops) and remounts it fresh for the
/// new user. All the normal boot code (store open, `hydrate_from_store`,
/// `SetAuth`) re-runs under the new namespace — no bespoke teardown.
///
/// The ambient [`namespace`] is set from the active id on **every** render,
/// synchronously before the keyed children (re)mount, so their stores read the
/// correct prefix the moment they open.
#[component]
pub fn AccountsProvider(children: Element) -> Element {
    let accounts = use_signal(Accounts::default);
    let ready = use_signal(|| false);

    use_future(move || async move {
        let loaded = AccountsStore::load().await;
        let mut accounts = accounts;
        let mut ready = ready;
        halogen_ui_logging::info!(
            users = loaded.users.len(),
            active = ?loaded.active_user_id,
            "Account registry loaded"
        );
        accounts.set(loaded);
        ready.set(true);
    });

    use_context_provider(|| accounts);

    // Mirror the active account into the ambient namespace on every render,
    // before the keyed subtree below mounts/remounts (its stores read the
    // namespace as they open). Reading the signal here also subscribes us, so a
    // switch re-renders this provider → updates the namespace → re-keys the
    // subtree. The kind rides along: embedded accounts live under `e{id}`.
    let active = accounts.read().active_key();
    namespace::set_active(
        active.map(|k| k.id),
        active.is_some_and(|k| k.is_embedded()),
        active.map(|k| k.server).unwrap_or(0),
    );

    if !ready() {
        return rsx! {
            super::LoadingSplash {}
        };
    }

    // Key the subtree on the active user so a switch REMOUNTS it (tears down the
    // worker/stores/player, then re-creates them under the new namespace).
    //
    // Subtlety: dioxus-core's `diff_node` diffs two same-template nodes in place —
    // a `key` only forces a remount when the node is a keyed item in a *list*
    // (the iterator/keyed-children diff). So the keyed wrapper is emitted through a
    // one-item iterator, not as a plain root element. `display: contents` keeps the
    // wrapper out of layout. Children move through `take()` (no `Element: Clone`).
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
