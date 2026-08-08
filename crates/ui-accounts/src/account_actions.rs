//! Account lifecycle actions (switch / sign out / wipe) shared by the navbar
//! switcher and the Settings accounts section. The thin registry *accessor* lives
//! in `hooks::use_accounts`; these are the heavy multi-step UI actions.
//!
//! These are complete UI actions: they perform the state change *and* surface its
//! outcome (a toast, a redirect to a prefilled login, or first-boot setup), so the
//! call sites are one-liners and don't duplicate outcome handling.
//!
//! Storage model: every user's full config lives at its own namespaced key
//! (`halogen.u{id}.client_config`), kept current by that user's own session saves
//! — so a switch never copies config. It re-mints the target's cookie via
//! `/auth/refresh` and flips `active_user_id`, which remounts the data subtree
//! under the target's namespace (see `AccountsProvider`).
//!
//! Concurrency note: flipping `active_user_id` triggers that remount, tearing down
//! the component whose handler spawned the action. A `Signal::set` only *schedules*
//! the re-render (code up to the next `.await` still runs), so every action does
//! its persistence + navigation **before** the remount-triggering `accounts.set`,
//! which is the last statement with no `.await` after it.

use dioxus::prelude::*;
use halogen_api::{ApiError, TokenData};

use crate::accounts::{Accounts, AccountsStore};
use halogen_ui_commands::Command;
use halogen_ui_config::{
    AccountKey, ClientConfig, ClientConfigStore, ListViewStore, api_client_from,
};
use halogen_ui_toast::ToastHandle;
/// Outcome of a `/auth/refresh` attempt against a stored token.
enum RefreshError {
    /// Token rejected (401) — needs interactive re-auth.
    Expired,
    /// Missing creds or a transport error.
    Unusable,
}

/// Re-mint a user's `auth_media` cookie + API token via `/auth/refresh`, given
/// their stored config. Returns the fresh token on success. (The cookie is
/// `HttpOnly`, so refresh is the only way to align it with a user on a switch.)
async fn refresh_token(config: &ClientConfig) -> Result<String, RefreshError> {
    let (Some(server_url), Some(token)) = (config.server_url.clone(), config.access_token.clone())
    else {
        return Err(RefreshError::Unusable);
    };
    let Some(client) = api_client_from(Some(server_url.as_str()), Some(token.as_str())) else {
        return Err(RefreshError::Unusable);
    };
    match client.refresh(TokenData { token }).await {
        Ok(fresh) => Ok(fresh.token),
        Err(ApiError::Server { status: 401, .. }) => Err(RefreshError::Expired),
        Err(_) => Err(RefreshError::Unusable),
    }
}

/// Hot-swap the active user to `target`.
///
/// Re-mints the target's cookie + token and flips the active key (remounting the
/// data subtree under the target's namespace). On an expired stored token, flags
/// the account and routes to a prefilled login; on a transport failure, toasts. A
/// no-op when already active.
///
/// Remote accounts only in practice: an Embedded target's server may not even be
/// running, so `ui-state`'s kind-aware wrapper boots it and silently re-auths
/// with the stored local credentials instead of this refresh path (which it
/// still uses for remote targets).
pub async fn switch_account(
    mut accounts: Signal<Accounts>,
    toast: ToastHandle,
    target: AccountKey,
) {
    if accounts.peek().active_key() == Some(target) {
        return;
    }
    let mut cfg = ClientConfigStore::load_for(target).await;
    match refresh_token(&cfg).await {
        Ok(fresh) => {
            // Persist the fresh token in the target's own namespace; the remount
            // then loads it. The outgoing user's config is already current at their
            // namespace — nothing to stash.
            cfg.access_token = Some(fresh);
            ClientConfigStore::save_for(target, &cfg).await;
            let mut reg = accounts.peek().clone();
            set_needs_reauth(&mut reg, target, false);
            reg.set_active(Some(target));
            AccountsStore::save(&reg).await;
            accounts.set(reg); // last: schedules the remount
        }
        Err(RefreshError::Expired) => {
            // Flag for re-auth and route to a prefilled login (active unchanged, so
            // no remount — the toast + nav below run normally).
            let mut reg = accounts.peek().clone();
            set_needs_reauth(&mut reg, target, true);
            AccountsStore::save(&reg).await;
            accounts.set(reg);
            toast.error("Session expired — please sign in again.");
            // Path-string nav (not `Route::Login`) so `ui-accounts` stays free of the
            // `Route` enum / router dependency, which lives up in `ui-views`. Matches `#[route("/auth/login")]`.
            let _ = navigator().replace("/auth/login");
        }
        Err(RefreshError::Unusable) => toast.error("Couldn't switch accounts."),
    }
}

/// Sign out `target`.
///
/// - Non-active account → removed in place (no remount).
/// - Active account with a successor → activates the successor (re-minting its
///   cookie when possible; otherwise it lands on a prefilled login).
/// - Last account → full device wipe + return to first-boot setup.
pub async fn sign_out_account(
    mut accounts: Signal<Accounts>,
    dispatch: &Coroutine<Command>,
    target: AccountKey,
) {
    let was_active = accounts.peek().active_key() == Some(target);

    if !was_active {
        let mut reg = accounts.peek().clone();
        reg.remove(target);
        AccountsStore::save(&reg).await;
        ClientConfigStore::clear_for(target).await;
        accounts.set(reg); // active unchanged → no remount
        return;
    }

    let successor = accounts
        .peek()
        .users
        .iter()
        .find(|u| u.key() != target)
        .map(|u| u.key());
    let Some(next) = successor else {
        // Last account — wipe everything and return to first-boot setup.
        wipe_all_accounts(accounts, dispatch).await;
        return;
    };

    // Activate the successor, re-minting its cookie/token when we can; on failure
    // it lands signed-out (prefilled login). (An Embedded successor lands on
    // needs_reauth here — its server is likely stopped — and heals via the
    // kind-aware boot path instead of the login page.)
    let mut next_cfg = ClientConfigStore::load_for(next).await;
    let needs_reauth = match refresh_token(&next_cfg).await {
        Ok(fresh) => {
            next_cfg.access_token = Some(fresh);
            false
        }
        Err(_) => {
            next_cfg.access_token = None;
            true
        }
    };
    ClientConfigStore::save_for(next, &next_cfg).await;
    ClientConfigStore::clear_for(target).await;

    let mut reg = accounts.peek().clone();
    reg.remove(target);
    set_needs_reauth(&mut reg, next, needs_reauth);
    reg.set_active(Some(next));
    AccountsStore::save(&reg).await;
    accounts.set(reg); // last: schedules the remount
}

/// Sign out of EVERY account and return to first-boot setup. Used when the last
/// account signs out and by Settings' "delete local data". Clears the worker's
/// stores (store/media/logs), the active user's list-view state, and — on web —
/// all of localStorage + Cache Storage, then resets the registry: clearing the
/// active user remounts the data subtree to the anon namespace.
///
/// The media + log purges go through the worker-INDEPENDENT `cache_purge` path
/// (awaited here), NOT the worker's `WipeLocal`: `accounts.set` below remounts and
/// hard-terminates the worker, which races — and can win — against a worker that
/// hasn't yet dequeued `WipeLocal`, leaving GBs of downloaded audio behind on a
/// device the user asked to wipe. `WipeLocal` is still dispatched so a surviving
/// worker also drops its own in-memory state, but completeness never depends on it.
pub async fn wipe_all_accounts(mut accounts: Signal<Accounts>, dispatch: &Coroutine<Command>) {
    // Dispatch `Command::WipeLocal` directly rather than through a `commands`
    // helper, so `ui-accounts` doesn't depend up on `ui-state`'s command layer.
    dispatch.send(Command::WipeLocal);
    // Clear EVERY known account's config + list-view state, not just the active
    // one. On web the storage purge below wipes all of localStorage, but on native
    // each user's `u{id}/` files (token + prefs + list views) must be removed
    // explicitly — otherwise other accounts' tokens linger on disk after a full
    // "delete local data". `clear_for` clears both the config and the sibling
    // list-view store for that id.
    let keys: Vec<AccountKey> = accounts.peek().users.iter().map(|u| u.key()).collect();
    for key in keys {
        ClientConfigStore::clear_for(key).await;
    }
    // Belt-and-suspenders: clear the active (ambient) namespace's list views too,
    // covering the edge where the active id isn't present in the registry list.
    ListViewStore::clear().await;
    // Worker-independent purges — guaranteed complete even if the worker is killed
    // mid-wipe: every account's downloaded audio + the captured device logs. Both
    // read the account registry for their segment list, so run them BEFORE the
    // registry reset below.
    halogen_ui_cache_purge::clear_audio().await;
    halogen_ui_cache_purge::clear_logs().await;
    purge_browser_storage().await;
    AccountsStore::save(&Accounts::default()).await;
    // Path-string nav (see note in `switch_account`); matches `#[route("/auth/login")]`.
    let _ = navigator().replace("/auth/login");
    accounts.set(Accounts::default()); // last: schedules the remount
}

/// Set (or clear) the re-auth flag on an account, if present.
fn set_needs_reauth(reg: &mut Accounts, key: AccountKey, value: bool) {
    if let Some(u) = reg.users.iter_mut().find(|u| u.key() == key) {
        u.needs_reauth = value;
    }
}

/// Purge ALL client IndexedDB databases and every Cache Storage entry (the PWA
/// service-worker asset caches) via the shared `cache_purge` failsafe helpers, so
/// there's a single copy of the wipe. No-op on native (the worker's `wipe_local`
/// plus the config-file reset cover that target — `cache_purge`'s native stubs are
/// likewise no-ops).
async fn purge_browser_storage() {
    halogen_ui_cache_purge::clear_all_storage().await;
    halogen_ui_cache_purge::clear_cached_assets().await;
}
