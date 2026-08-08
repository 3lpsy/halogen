//! `RootGuard` — the outermost in-router auth layout.

use dioxus::prelude::*;

use crate::Route;
use crate::components::RouteHistory;
use halogen_ui_logging as logging;
use halogen_ui_state::hooks::use_config;

/// The route an unauthenticated redirect displaced — the deep link (or the page
/// the user sat on when the token expired). Provided by [`RootGuard`]; consumed
/// (take + navigate) by the auth pages on success, so signing in lands back
/// where the user was headed instead of always on Home.
#[derive(Clone, Copy)]
pub struct PendingAuthRedirect(pub Signal<Option<Route>>);

impl PendingAuthRedirect {
    /// The stashed destination, cleared on read — falls back to Home.
    pub fn take_or_home(&self) -> Route {
        let mut inner = self.0;
        inner.take().unwrap_or(Route::Home {})
    }
}

/// Outermost route layout: redirects unauthenticated users to the login page
/// (server URL + credentials on one form), or to the embedded reconnect page
/// for embedded accounts. Runs inside the Router so `use_navigator` has
/// router context.
#[component]
pub fn RootGuard() -> Element {
    let config = use_config();
    let nav = use_navigator();
    let route: Route = use_route();

    // The deep-link stash for the redirect below (see `PendingAuthRedirect`).
    let pending_redirect = use_context_provider(|| PendingAuthRedirect(Signal::new(None)));

    // Shared two-entry route history (previous + current). The sidebar and dock
    // read it (via `use_active_nav_key`) to highlight the right item for
    // section/detail pages, and to inherit an episode page's origin section.
    // Provided here — the outermost in-router layout — so it spans every route.
    let mut route_history = use_context_provider(|| Signal::new(RouteHistory::default()));

    // Log every navigation + record it into the history. `use_reactive` so the
    // effect fires on each route change (not only when the redirect effect's
    // config dependency changes).
    use_effect(use_reactive!(|route| {
        logging::debug!(route = ?route, "Navigation");
        route_history.with_mut(|h| {
            if h.current.as_ref() != Some(&route) {
                h.previous = h.current.take();
                h.current = Some(route.clone());
            }
        });
    }));

    // `use_reactive!` on `route` so the guard re-evaluates on every navigation
    // (reading the captured `route` value alone would not subscribe the effect to
    // route changes — only the inner `config()` read would). The `config()` read
    // keeps it reactive to auth changes (token expiry) too.
    use_effect(use_reactive!(|route| {
        let cfg = config();
        let authed = cfg.is_authenticated();
        let on_auth_route = matches!(route, Route::Login {} | Route::EmbeddedServerSetup {});
        if !authed && !on_auth_route {
            // Remember where the user was headed so the auth page can land
            // back there after sign-in instead of discarding the deep link.
            let mut stash = pending_redirect.0;
            stash.set(Some(route.clone()));
            // Embedded accounts never see the password prompt — their
            // credentials live on disk; the embedded-setup page reconnects
            // (boot + silent sign-in) instead.
            if cfg.server_kind.is_embedded() {
                logging::info!("Not authenticated, redirecting to embedded reconnect");
                nav.replace(Route::EmbeddedServerSetup {});
            } else {
                // Login prefills the server URL when one is already known
                // (e.g. the token just expired).
                logging::info!("Not authenticated, redirecting to login");
                nav.replace(Route::Login {});
            }
        }
    }));

    // When a redirect is pending (unauthenticated on a guarded route), render the
    // splash rather than the guarded `Outlet` — the effect above commits the
    // `nav.replace` next frame, and showing the protected page for that one frame
    // would flash content the user isn't allowed to see.
    let cfg = config();
    let on_auth_route = matches!(route, Route::Login {} | Route::EmbeddedServerSetup {});
    if !cfg.is_authenticated() && !on_auth_route {
        return rsx! { halogen_ui_state::providers::LoadingSplash {} };
    }

    rsx! { Outlet::<Route> {} }
}
