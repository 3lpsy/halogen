//! Local profiles retain account/cache identity without credentials or network authentication.

use dioxus::prelude::*;

use halogen_webui_accounts::account_actions;
use halogen_webui_accounts::accounts::{Accounts, AccountsStore, StoredAccount};
use halogen_webui_commands::Command;
use halogen_webui_component_toast::ToastHandle;
use halogen_webui_config::{AccountKey, ClientConfigStore, ServerKind};
use halogen_webui_logging::{info, warn};
use halogen_wire::UserStoreData;

use crate::embedded;

struct EmbeddedAuth {
    key: AccountKey,
    token: String,
    username: String,
    base: String,
    is_admin: bool,
}

async fn authenticate(username: Option<&str>) -> Result<EmbeddedAuth, String> {
    let profile = embedded::profile(username).await?;
    Ok(EmbeddedAuth {
        key: AccountKey::embedded(profile.id),
        token: String::new(),
        username: profile.username,
        base: profile.base,
        is_admin: profile.is_admin,
    })
}
async fn authenticate_admin() -> Result<EmbeddedAuth, String> {
    authenticate(None).await
}
async fn authenticate_user(username: &str) -> Result<EmbeddedAuth, String> {
    authenticate(Some(username)).await
}

/// Persist a completed authentication into the account's `e{id}` namespace and
/// return the updated registry, with the account active. The caller sets the
/// registry signal LAST (after any navigation) — that schedules the keyed
/// remount, same contract as the login page.
async fn persist_activation(
    accounts: Signal<Accounts>,
    auth: EmbeddedAuth,
) -> Result<Accounts, String> {
    // Preserve any prior session's prefs; the load-time overlay applies too.
    let mut cfg = ClientConfigStore::load_for(auth.key).await;
    cfg.server_url = Some(auth.base);
    cfg.server_kind = ServerKind::Embedded;
    cfg.server_setup = true;
    cfg.access_token = Some(auth.token);
    // Imported profiles retain their database privileges.
    cfg.is_admin = auth.is_admin;
    ClientConfigStore::save_for(auth.key, &cfg).await;

    let mut reg = accounts.peek().clone();
    reg.upsert(StoredAccount {
        id: auth.key.id,
        username: auth.username,
        needs_reauth: false,
        kind: ServerKind::Embedded,
        // Embedded = one server per device; no server component (see AccountKey).
        server: 0,
    });
    reg.set_active(Some(auth.key));
    AccountsStore::save(&reg).await;
    info!("Embedded account activated");
    Ok(reg)
}

/// Validate the selected local profile. The empty result preserves the legacy auth-ready marker.
pub async fn silent_relogin(accounts: Signal<Accounts>) -> Result<String, String> {
    let active = accounts
        .peek()
        .active_key()
        .filter(|k| k.is_embedded())
        .ok_or_else(|| "No active embedded account".to_string())?;
    let username = accounts
        .peek()
        .users
        .iter()
        .find(|u| u.key() == active)
        .map(|u| u.username.clone())
        .ok_or_else(|| "Active embedded account missing from the registry".to_string())?;
    let auth = authenticate_user(&username).await?;
    if auth.key != active {
        return Err(format!(
            "Embedded re-auth mismatch: signed in as user {} but the active account is {}",
            auth.key.id, active.id
        ));
    }
    Ok(auth.token)
}

/// Open the selected local profile, or bootstrap the first profile for a new library.
pub async fn prepare_embedded_activation(accounts: Signal<Accounts>) -> Result<Accounts, String> {
    let reconnect_as = accounts
        .peek()
        .active_key()
        .filter(|k| k.is_embedded())
        .and_then(|key| {
            accounts
                .peek()
                .users
                .iter()
                .find(|u| u.key() == key)
                .map(|u| u.username.clone())
        });
    let auth = match reconnect_as {
        Some(username) => authenticate_user(&username).await?,
        None => authenticate_admin().await?,
    };
    persist_activation(accounts, auth).await
}

/// Create a local profile using the active profile's authorized admin client.
pub async fn create_embedded_user(
    accounts: Signal<Accounts>,
    username: &str,
) -> Result<Accounts, String> {
    let trimmed = username.trim().to_lowercase();
    if trimmed.len() < 3 {
        return Err("Username must be at least 3 characters".to_string());
    }

    // Catch saved duplicates before the database uniqueness check.
    if accounts
        .peek()
        .users
        .iter()
        .any(|u| u.kind.is_embedded() && u.username == trimmed)
    {
        return Err(format!("'{trimmed}' already exists — switch to it instead"));
    }

    // The creator's session performs the admin create call.
    let active_cfg = ClientConfigStore::load().await;
    if !active_cfg.server_kind.is_embedded() {
        return Err("Adding embedded users requires an active embedded account".to_string());
    }
    let admin_client = active_cfg
        .api_client()
        .ok_or_else(|| "No embedded session to create the user with".to_string())?;

    let password = embedded::new_password();
    admin_client
        .create_user(UserStoreData {
            username: trimmed.clone(),
            password: password.clone(),
            password_confirm: password,
            is_admin: Some(true),
        })
        .await
        .map_err(|e| format!("Couldn't create the user: {e}"))?;
    let auth = authenticate_user(&trimmed).await?;
    persist_activation(accounts, auth).await
}

/// Verify imported profiles can be selected without generating credentials.
pub async fn align_imported_users(created_usernames: &[String]) {
    for username in created_usernames {
        if let Err(e) = embedded::profile(Some(username)).await {
            warn!("Couldn't align imported user '{username}' for local use: {e}");
        }
    }
}

/// Switch local profile bindings or refresh a remote session, preserving each account's cache namespace.
pub async fn switch_account_smart(
    mut accounts: Signal<Accounts>,
    toast: ToastHandle,
    target: AccountKey,
) {
    if accounts.peek().active_key() == Some(target) {
        return;
    }
    let was_embedded = accounts
        .peek()
        .active_key()
        .is_some_and(|k| k.is_embedded());

    if target.is_embedded() {
        let Some(username) = accounts
            .peek()
            .users
            .iter()
            .find(|u| u.key() == target)
            .map(|u| u.username.clone())
        else {
            toast.error("That account is no longer registered.");
            return;
        };
        let outcome = async {
            let auth = authenticate_user(&username).await?;
            if auth.key != target {
                return Err(format!(
                    "Signed in as a different user (id {}) than the selected account (id {})",
                    auth.key.id, target.id
                ));
            }
            persist_activation(accounts, auth).await
        }
        .await;
        match outcome {
            Ok(reg) => accounts.set(reg), // last: schedules the remount
            Err(e) => {
                warn!("Embedded switch failed: {e}");
                toast.error("Couldn't switch to the embedded account.");
            }
        }
        return;
    }

    account_actions::switch_account(accounts, toast, target).await;
    // Free the in-process server once the active account is no longer
    // embedded. Detached (`spawn_forever`): the switch just scheduled a
    // remount that tears down the caller's scope.
    if was_embedded && accounts.peek().active_key() == Some(target) {
        let _ = dioxus::core::spawn_forever(async {
            embedded::stop().await;
        });
    }
}

/// Sign out an account while retaining the local library for a later session.
pub async fn sign_out_account_smart(
    accounts: Signal<Accounts>,
    dispatch: &Coroutine<Command>,
    target: AccountKey,
) {
    account_actions::sign_out_account(accounts, dispatch, target).await;

    let embedded_still_active = accounts
        .peek()
        .active_key()
        .is_some_and(|k| k.is_embedded());
    if target.is_embedded() && !embedded_still_active {
        let _ = dioxus::core::spawn_forever(async {
            embedded::stop().await;
        });
    }
}
