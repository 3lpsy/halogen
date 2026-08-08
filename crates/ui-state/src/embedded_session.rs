//! Kind-aware session operations. `ui-accounts` owns the remote flows
//! (refresh-based switch, sign-out, wipe) and stays free of any embedded
//! knowledge; these wrappers route embedded targets through the supervisor
//! (boot + silent credential login + self-heal) and delegate remote targets
//! straight through. Views call these, never `ui-accounts` directly, once an
//! embedded account can exist.
//!
//! Every embedded user's password lives in the server's `secrets.json`
//! (generated, never typed): the seeded `local` admin from first boot, plus
//! any account created through [`create_embedded_user`]. Sign-in is therefore
//! always per-USERNAME — switching to a second embedded account authenticates
//! as *that* user, not the admin.

use dioxus::prelude::*;

use halogen_api::{ApiClient, ApiError};
use halogen_ui_accounts::account_actions;
use halogen_ui_accounts::accounts::{Accounts, AccountsStore, StoredAccount, jwt_sub};
use halogen_ui_commands::Command;
use halogen_ui_config::{AccountKey, ClientConfigStore, ServerKind, api_client_from};
use halogen_ui_logging::{info, warn};
use halogen_ui_toast::ToastHandle;
use halogen_wire::{LoginData, UserStoreData};

use crate::embedded;
use crate::embedded::EmbeddedCredentials;

/// A completed embedded authentication: the account key (from the token's
/// `sub`), the fresh token, the username that logged in, and the live base URL.
struct EmbeddedAuth {
    key: AccountKey,
    token: String,
    username: String,
    base: String,
}

/// Which self-heal a 401 triggers: the seeded admin recovers via
/// `recover_admin` (which also ADOPTS a renamed admin row and can seed an
/// empty table); everyone else rotates their own row via `recover_user`.
#[derive(Clone, Copy)]
enum Recovery {
    Admin,
    User,
}

/// Boot the embedded server (if needed) and log `creds` in, self-healing a
/// drifted password on a 401 per `recovery`. Persists nothing.
async fn login_with(
    base: &str,
    client: &ApiClient,
    creds: EmbeddedCredentials,
    recovery: Recovery,
) -> Result<EmbeddedAuth, String> {
    let attempt = |username: String, password: String| {
        let client = &client;
        async move {
            client
                .login(LoginData { username, password })
                .await
                .map(|t| t.token)
        }
    };

    let (token, username) = match attempt(creds.username.clone(), creds.password.clone()).await {
        Ok(token) => (token, creds.username),
        Err(ApiError::Server { status: 401, .. }) | Err(ApiError::Validation(_)) => {
            // Credential drift (stale secrets file / out-of-band change):
            // rotate the password directly in the DB and retry once.
            warn!("Embedded credentials rejected — running recovery");
            let fresh = match recovery {
                Recovery::Admin => embedded::recover_admin().await?,
                Recovery::User => embedded::recover_user(&creds.username).await?,
            };
            let token = attempt(fresh.username.clone(), fresh.password.clone())
                .await
                .map_err(|e| format!("Embedded sign-in failed after recovery: {e}"))?;
            (token, fresh.username)
        }
        Err(e) => return Err(format!("Embedded sign-in failed: {e}")),
    };

    let id = jwt_sub(&token).ok_or_else(|| "Embedded sign-in: malformed token".to_string())?;
    Ok(EmbeddedAuth {
        key: AccountKey::embedded(id),
        token,
        username,
        base: base.to_string(),
    })
}

/// Ensure the server is up and serving, and build a client for its base URL.
async fn ready_client() -> Result<(String, ApiClient), String> {
    let base = embedded::ensure_started()?;
    embedded::wait_ready().await?;
    let client = api_client_from(Some(&base), None)
        .ok_or_else(|| "Invalid embedded server URL".to_string())?;
    Ok((base, client))
}

/// Authenticate the SEEDED admin (first-boot onboarding / reconnect). Uses
/// `recover_admin` semantics indirectly: `credentials()` reads the admin entry
/// and the 401 path rotates it.
async fn authenticate_admin() -> Result<EmbeddedAuth, String> {
    let (base, client) = ready_client().await?;
    let creds = embedded::credentials()?;
    login_with(&base, &client, creds, Recovery::Admin).await
}

/// Authenticate a SPECIFIC embedded user by username (account switch, silent
/// re-login for the active account).
async fn authenticate_user(username: &str) -> Result<EmbeddedAuth, String> {
    let (base, client) = ready_client().await?;
    let creds = embedded::credentials_for(username)?;
    login_with(&base, &client, creds, Recovery::User).await
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
    // Every embedded user is created as an admin (the add-user form locks the
    // toggle on), and the seeded account IS the admin.
    cfg.is_admin = true;
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

/// Fresh token for the ACTIVE embedded account (the worker's silent re-auth
/// path) — authenticates as that account's own username, never the admin.
/// Persists nothing; the caller owns the config signal.
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

/// Full embedded sign-in: first-boot onboarding and the reconnect path.
/// When an embedded account is already ACTIVE in the registry (RootGuard sent
/// its owner here to reconnect after a failed silent re-auth), sign back in as
/// THAT user — reconnecting must never silently swap a non-admin account for
/// the seeded admin. Fresh setups (no active embedded account) provision via
/// the admin. Returns the updated registry for the caller to
/// `accounts.set(...)` LAST (after any navigation).
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

/// Create a NEW embedded user (admin-only server endpoint; the active account
/// is an embedded admin) and activate them: generate + store their password,
/// register them server-side, sign them in, and return the updated registry
/// (caller sets it last). All embedded users are admins by policy — the
/// add-user form shows the toggle locked on.
pub async fn create_embedded_user(
    accounts: Signal<Accounts>,
    username: &str,
) -> Result<Accounts, String> {
    let trimmed = username.trim().to_lowercase();
    if trimmed.len() < 3 {
        return Err("Username must be at least 3 characters".to_string());
    }

    // Fast local guards before touching anything: a known name (stored
    // credentials or a registered embedded account) must not reach
    // `remember_user` — clobbering a healthy user's secret on a doomed create
    // was the failure mode here.
    if embedded::has_credentials(&trimmed)
        || accounts
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

    let (base, client) = ready_client().await?;
    // Order matters: create server-side FIRST, persist the secret only on
    // success — a rejected create (duplicate on the server, validation) must
    // leave secrets.json untouched.
    let creds = embedded::generate_credentials(&trimmed);
    admin_client
        .create_user(UserStoreData {
            username: creds.username.clone(),
            password: creds.password.clone(),
            password_confirm: creds.password.clone(),
            is_admin: Some(true),
        })
        .await
        .map_err(|e| format!("Couldn't create the user: {e}"))?;
    embedded::remember_user(&creds)?;

    let auth = login_with(&base, &client, creds, Recovery::User).await?;
    persist_activation(accounts, auth).await
}

/// After a DB import on the embedded server: users the import created got
/// RANDOM passwords the app doesn't know — rotate each one into the secrets
/// store so switching to them silently just works. Best-effort per user.
pub async fn align_imported_users(created_usernames: &[String]) {
    for username in created_usernames {
        if let Err(e) = embedded::recover_user(username).await {
            warn!("Couldn't align imported user '{username}' for silent login: {e}");
        }
    }
}

/// Kind-aware account switch. Remote targets take the ordinary refresh path;
/// embedded targets boot the server and re-auth with the TARGET's stored
/// credentials (their server may not even be running — refresh could never
/// work). Stops the embedded server once no embedded account is active.
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

/// Kind-aware sign-out. Delegates to the ordinary flow, then stops the
/// embedded server once no embedded account is active. The library is NEVER
/// destroyed here — even signing out the last account only wipes client-side
/// data; the embedded server's library stays on disk and "Use Embedded
/// Server" reconnects to it. Deleting the library is an explicit act (the
/// cache-control page's cards).
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
