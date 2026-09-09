//! Shared account actions include persistence, feedback, and navigation. Config remains in each account namespace;
//! switching refreshes auth then remounts the subtree. Finish persistence/navigation before the final accounts.set and
//! never await afterward, because that remount cancels the originating scope.

use dioxus::prelude::*;
use halogen_apiclient::{ApiError, TokenData};

use crate::accounts::{Accounts, AccountsStore};
use halogen_webui_commands::Command;
use halogen_webui_component_toast::ToastHandle;
use halogen_webui_config::{
    AccountKey, ClientConfig, ClientConfigStore, ListViewStore, api_client_from,
};
/// Outcome of a `/auth/refresh` attempt against a stored token.
enum RefreshError {
    /// Token rejected (401/403), requiring interactive re-auth.
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
        Err(error) => Err(classify_refresh_error(error)),
    }
}

fn classify_refresh_error(error: ApiError) -> RefreshError {
    match error {
        ApiError::Server {
            status: 401 | 403, ..
        } => RefreshError::Expired,
        _ => RefreshError::Unusable,
    }
}

/// Refresh remote auth and activate the target namespace, or flag rejected auth for prefilled login and toast transport
/// failures. Already-active is a no-op. Local targets use a kind-aware wrapper that boots and silently authenticates
/// from disk credentials.
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

/// Remove inactive accounts in place. Signing out the active account selects a successor and refreshes it when
/// possible; removing the last account wipes device data and returns to setup.
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

    // A failed refresh must not invalidate the successor's saved credentials.
    let mut next_cfg = ClientConfigStore::load_for(next).await;
    let previously_expired = accounts
        .peek()
        .users
        .iter()
        .find(|user| user.key() == next)
        .is_some_and(|user| user.needs_reauth);
    let refreshed = refresh_token(&next_cfg).await;
    let needs_reauth = apply_successor_refresh(&mut next_cfg, previously_expired, refreshed);
    ClientConfigStore::save_for(next, &next_cfg).await;
    ClientConfigStore::clear_for(target).await;

    let mut reg = accounts.peek().clone();
    reg.remove(target);
    set_needs_reauth(&mut reg, next, needs_reauth);
    reg.set_active(Some(next));
    AccountsStore::save(&reg).await;
    accounts.set(reg); // last: schedules the remount
}

/// Wipe all account data before resetting to anonymous setup. Await worker-independent media/log purges because remount
/// can terminate the worker before WipeLocal executes. Still dispatch WipeLocal for surviving in-memory state; wipe
/// completeness must never depend on it.
pub async fn wipe_all_accounts(mut accounts: Signal<Accounts>, dispatch: &Coroutine<Command>) {
    // Dispatch `Command::WipeLocal` directly rather than through a `commands`
    // helper, so `ui-accounts` doesn't depend up on `ui-state`'s command layer.
    dispatch.send(Command::WipeLocal);
    // Clear EVERY known account's config + list-view state, not the active one. On web the storage purge below wipes
    // all of localStorage, but on native each user's `u{id}/` files (token + prefs + list views) must be removed
    // explicitly, otherwise other accounts' tokens linger on disk after a full "delete local data". `clear_for` clears
    // both the config and the sibling list-view store for that id.
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
    halogen_webui_cache_purge::clear_audio().await;
    halogen_webui_cache_purge::clear_logs().await;
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

/// Purge ALL client IndexedDB databases and every Cache Storage entry (the PWA service-worker asset caches) via the
/// shared `cache_purge` failsafe helpers, so there's a single copy of the wipe. No-op on native (the worker's
/// `wipe_local` plus the config-file reset cover that target, `cache_purge`'s native stubs are likewise no-ops).
async fn purge_browser_storage() {
    halogen_webui_cache_purge::clear_all_storage().await;
    halogen_webui_cache_purge::clear_cached_assets().await;
}

/// Only an authoritative rejection clears credentials; transient failure preserves prior auth state.
fn apply_successor_refresh(
    config: &mut ClientConfig,
    previously_expired: bool,
    refreshed: Result<String, RefreshError>,
) -> bool {
    match refreshed {
        Ok(token) => {
            config.access_token = Some(token);
            false
        }
        Err(RefreshError::Expired) => {
            config.access_token = None;
            true
        }
        Err(RefreshError::Unusable) => previously_expired || config.access_token.is_none(),
    }
}

#[cfg(test)]
#[path = "account_actions_tests.rs"]
mod tests;
