use dioxus::prelude::*;
use halogen_api::{ApiClient, LoginData};

use super::card::AuthCard;
use super::{AuthErrorContext, auth_error_message};
use crate::Route;
use crate::root_guard::PendingAuthRedirect;
use halogen_ui_accounts::accounts::{AccountsStore, StoredAccount, jwt_sub};
use halogen_ui_config::ClientConfigStore;
use halogen_ui_logging::info;
use halogen_ui_state::hooks::{use_accounts, use_config, use_toast};
use halogen_ui_toast::{ApiResultExt, ToastPolicy};

/// Default value for the server URL input when no server is known yet.
///
/// On wasm we dogfood the web app served from the same origin as the API, so
/// prefill the origin the app was pulled from. On native there's no meaningful
/// origin, so start empty.
fn default_server_url() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

/// The single connect page (iOS/Android parity): server URL + credentials
/// entered together — normalize → scheme check → health probe → login.
#[component]
pub fn Login() -> Element {
    let pending_redirect = use_context::<PendingAuthRedirect>();
    let accounts = use_accounts();
    let config_sig = use_config();
    // Prefill the server from the active config (an expired session, or a
    // signed-out relaunch still knows its server), else the page's own origin.
    let initial_url = config_sig
        .peek()
        .server_url
        .clone()
        .unwrap_or_else(default_server_url);
    // Prefill the username when we landed here because a stored session expired:
    // a failed switch flags that account `needs_reauth`.
    let initial_user = accounts
        .peek()
        .users
        .iter()
        .find(|u| u.needs_reauth)
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let url_prefilled = !initial_url.is_empty();
    let mut server_url = use_signal(|| initial_url);
    let mut username = use_signal(|| initial_user);
    let mut password = use_signal(String::new);
    let error = use_signal(|| None::<String>);
    let loading = use_signal(|| false);
    let nav = use_navigator();
    let toast = use_toast();

    let onsubmit = move |e: FormEvent| {
        e.prevent_default();
        // Normalize like the native connect pages: stray whitespace and
        // trailing slashes are typos, not intent.
        let entered = server_url().trim().trim_end_matches('/').to_string();
        let user = username();
        let pass = password();
        let mut err_signal = error;
        let mut loading_signal = loading;
        let mut accounts = accounts;
        let mut config_sig = config_sig;
        loading_signal.set(true);

        let _ = spawn(async move {
            // Single bail path: surface `msg` inline and drop out of the in-flight
            // state. Every early return below funnels through this so the
            // `loading.set(false)` is written once.
            let mut bail = move |msg: String| {
                err_signal.set(Some(msg));
                loading_signal.set(false);
            };

            let Ok(parsed) = entered.parse::<url::Url>() else {
                bail("Invalid URL".into());
                return;
            };
            // Only http(s): the parsed base builds every API/media URL, so a
            // `file:`/`ftp:`/`javascript:` scheme must never be stored.
            if !halogen_utils::constants::ALLOWED_SERVER_URL_SCHEMES.contains(&parsed.scheme()) {
                bail("Server URL must start with http:// or https://".into());
                return;
            }
            let server_url = entered;

            let client = ApiClient::new(parsed);
            // Reachability first: a wrong URL should read as "can't reach the
            // server", not as a login failure.
            if let Err(e) = client.health().await {
                bail(auth_error_message(&e, AuthErrorContext::Setup));
                return;
            }

            // Foreground policy: validation/invalid-credentials/unreachable stay
            // inline (handled below); only unexpected runtime errors (5xx/decode)
            // additionally toast.
            let result = client
                .login(LoginData {
                    username: user.clone(),
                    password: pass,
                })
                .await
                .report(&toast, ToastPolicy::Foreground);

            match result {
                Ok(token_data) => {
                    info!("Login successful at {server_url}");
                    client.set_token(Some(token_data.token.clone()));
                    // The id keys this user's storage namespace; without it we
                    // can't partition data, so a malformed token is a hard error.
                    let Some(id) = jwt_sub(&token_data.token) else {
                        bail("Login failed: malformed token".into());
                        return;
                    };
                    // Write this user's config into their own namespace. Start from
                    // any prior session (preserves their prefs), then apply the
                    // server URL + fresh auth. `save_for` (not the ambient `save`)
                    // because the ambient namespace is still the pre-login one until
                    // the remount below.
                    // Scope this account's identity + storage namespace by its
                    // server, so the same user id on a different remote server
                    // can't collapse onto this one's namespace/outbox.
                    let server = halogen_ui_config::server_hash(&server_url);
                    let key = halogen_ui_config::AccountKey::remote(id, server);
                    let mut cfg = ClientConfigStore::load_for(key).await;
                    // Learn username + admin status (best-effort). A transient
                    // `get_user` failure must NOT downgrade the account to non-admin
                    // — keep the prior known `is_admin` so a blip right after auth
                    // doesn't silently strip the admin UI until the next login.
                    let me = client.get_user(id).await.ok();
                    let username = me.as_ref().map(|m| m.username.clone()).unwrap_or(user);
                    let is_admin = me.as_ref().map(|m| m.is_admin).unwrap_or(cfg.is_admin);
                    cfg.server_url = Some(server_url);
                    cfg.server_kind = halogen_ui_config::ServerKind::Remote;
                    cfg.server_setup = true;
                    cfg.access_token = Some(token_data.token);
                    cfg.is_admin = is_admin;
                    ClientConfigStore::save_for(key, &cfg).await;

                    // Register + activate. Flipping `active_user_id` remounts the
                    // data subtree under this user's namespace (worker auths, stores
                    // reopen; the cookie was set by the login response). The `set` is
                    // last with no `.await` after it, so persistence + navigation
                    // finish before the remount tears this page down.
                    let mut reg = accounts.peek().clone();
                    reg.upsert(StoredAccount {
                        id,
                        username,
                        needs_reauth: false,
                        kind: halogen_ui_config::ServerKind::Remote,
                        server,
                    });
                    reg.set_active(Some(key));
                    AccountsStore::save(&reg).await;

                    // Reflect the new auth in the live config immediately. Re-login
                    // of the SAME active user doesn't change `active_user_id`, so the
                    // data subtree does NOT remount — without this the worker +
                    // RootGuard would keep the old (unauthenticated) config and loop.
                    // A *different* user instead remounts via `accounts.set` and
                    // reloads this from storage.
                    config_sig.set(cfg);
                    // Land back on the deep link the auth redirect displaced
                    // (falls back to Home).
                    let _ = nav.replace(pending_redirect.take_or_home());
                    accounts.set(reg);
                }
                Err(e) => {
                    bail(auth_error_message(&e, AuthErrorContext::Login));
                }
            }
        });
    };

    // Standalone mode: offered only when this build carries the in-process
    // server (native desktop/mobile). Rendered as the card footer so the
    // primary Connect flow stays visually first.
    let embedded_footer = halogen_ui_state::embedded::available().then(|| {
        rsx! {
            div { class: "divider text-xs text-muted my-4", "or" }
            button {
                r#type: "button",
                class: "btn btn-ghost btn-sm w-full",
                onclick: move |_| {
                    let _ = nav.replace(Route::EmbeddedServerSetup {});
                },
                "Use Embedded Server"
            }
        }
    });

    rsx! {
        AuthCard {
            subtitle: "Connect to your server",
            max_width: "max-w-md",
            error,
            loading,
            submit_label: "Connect",
            pending_label: "Connecting...",
            onsubmit,
            footer: embedded_footer,
            input {
                "aria-label": "Server URL",
                class: "input input-bordered w-full",
                r#type: "url",
                placeholder: "https://your-server.example.com",
                value: "{server_url}",
                oninput: move |e| server_url.set(e.value()),
                autofocus: !url_prefilled,
            }
            input {
                "aria-label": "Username",
                class: "input input-bordered w-full",
                r#type: "text",
                placeholder: "Username",
                // Usernames are lowercase; stop mobile keyboards from
                // capitalizing/"correcting" the first thing typed on this page.
                autocapitalize: "none",
                "autocorrect": "off",
                spellcheck: "false",
                autocomplete: "username",
                value: "{username}",
                oninput: move |e| username.set(e.value()),
                autofocus: url_prefilled,
            }
            input {
                "aria-label": "Password",
                class: "input input-bordered w-full",
                r#type: "password",
                placeholder: "Password",
                value: "{password}",
                oninput: move |e| password.set(e.value()),
            }
        }
    }
}
