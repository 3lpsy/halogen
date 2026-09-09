use crate::Route;
use dioxus::prelude::*;
pub use halogen_webui_component_navigation::{nav_icon, nav_items, nav_label};
use halogen_webui_config::{BuiltinNav, NavKey};
/// Two-entry route history (previous + current), provided as a `Signal<RouteHistory>` by `RootGuard` and updated on
/// every navigation. Active- nav resolution needs the *previous* route because some pages (EpisodeDetail) have no
/// section of their own and instead light up the nav item for wherever they were opened from (the back target).
#[derive(Clone, Default, PartialEq)]
pub struct RouteHistory {
    pub previous: Option<Route>,
    pub current: Option<Route>,
}

/// Map detail/form routes to their parent navigation section. Return None for shell/auth routes and EpisodeDetail,
/// whose highlight comes from the previous route via active_nav_key.
pub fn route_section(route: &Route) -> Option<NavKey> {
    use BuiltinNav as B;
    let key = |n| Some(NavKey::Builtin(n));
    match route {
        Route::Queue {} => key(B::Queue),
        Route::Latest {} => key(B::Latest),
        Route::Podcasts {}
        | Route::PodcastCreate {}
        | Route::PodcastDetail { .. }
        | Route::PodcastMetadata { .. }
        | Route::PodcastEdit { .. }
        | Route::PodcastConfigCreate { .. }
        | Route::PodcastConfigEdit { .. }
        | Route::PodcastAutoPlaylists { .. } => key(B::Podcasts),
        Route::Playlists {}
        | Route::PlaylistCreate {}
        | Route::PlaylistDetail { .. }
        | Route::PlaylistEdit { .. }
        | Route::PlaylistReorderBy { .. } => key(B::Playlists),
        Route::Downloads {} => key(B::Downloads),
        Route::Discover {} | Route::DiscoverDetail { .. } | Route::DiscoverPodcast { .. } | Route::DiscoverEpisode { .. } => key(B::Discover),
        Route::History {} => key(B::History),
        Route::Settings {}
        | Route::SettingsPlayback {}
        | Route::SettingsDownloads {}
        | Route::SettingsUi {}
        | Route::SettingsAccounts {}
        | Route::AddEmbeddedUser {}
        | Route::SettingsServer {}
        | Route::SettingsPodcasts {}
        | Route::ViewConfig {}
        | Route::ConfigOverridesEdit {}
        | Route::ConfigureDock {}
        | Route::ConfigureSwipes {}
        | Route::UserEdit { .. }
        | Route::AdminUsers {}
        | Route::AdminUserCreate {}
        | Route::AdminUserEdit { .. }
        // Server Errors is reached from Settings → Server, like AdminUsers.
        | Route::ServerErrors {} => key(B::Settings),
        Route::DeviceLogs {} => key(B::DeviceLogs),
        Route::ServerLogs {} => key(B::ServerLogs),
        Route::Polling {} | Route::Logs {} => key(B::Polling),
        // No home section — resolved from the previous route (EpisodeDetail) or
        // simply unhighlighted (chrome / auth).
        Route::EpisodeDetail { .. }
        | Route::EpisodeMetadata { .. }
        | Route::EpisodePlaylists { .. }
        | Route::BulkEpisodePlaylists { .. }
        | Route::Menu {}
        | Route::Home {}
        | Route::Login {}
        | Route::EmbeddedServerSetup {}
        // Standalone failsafe pages — no navbar renders them, but the match is exhaustive.
        | Route::CacheControl {}
        | Route::NotFound {} => None,
    }
}

/// Which nav key should appear active, given the current route, the previous one, and the set of pinned playlist ids.
/// `EpisodeDetail` has no section of its own, so it inherits the section of the page it was opened from. A pinned
/// playlist's own detail page lights *its pin* (not the generic Playlists item); an unpinned playlist detail folds into
/// Playlists via [`route_section`]. Shared by the sidebar and dock so the two can't drift.
pub fn active_nav_key(current: &Route, previous: Option<&Route>, pinned: &[i32]) -> Option<NavKey> {
    let resolve = |route: &Route| match route {
        Route::PlaylistDetail { id } if pinned.contains(id) => Some(NavKey::Pin(*id)),
        other => route_section(other),
    };
    match current {
        Route::EpisodeDetail { .. } => previous.and_then(resolve),
        other => resolve(other),
    }
}

/// Reactive hook: the active nav key for the current location. Reads the current route, the shared [`RouteHistory`],
/// and the pinned playlists so the sidebar and dock highlight the same item (including a pinned playlist's own detail
/// page). When the `RouteHistory` context is absent (e.g. a nav component mounted standalone in a unit test) it
/// degrades to current-route-only.
pub fn use_active_nav_key() -> Option<NavKey> {
    let current = use_route::<Route>();
    let previous =
        try_consume_context::<Signal<RouteHistory>>().and_then(|h| h.read().previous.clone());
    let pinned = halogen_webui_hooks::use_config()
        .read()
        .nav
        .pinned_playlists
        .clone();
    active_nav_key(&current, previous.as_ref(), &pinned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use halogen_webui_config::{ClientConfig, NavConfig, PlaybackPrefs};

    #[test]
    fn podcast_pages_fold_into_podcasts_section() {
        let k = Some(NavKey::Builtin(BuiltinNav::Podcasts));
        assert_eq!(route_section(&Route::Podcasts {}), k);
        assert_eq!(route_section(&Route::PodcastDetail { id: 7 }), k);
        assert_eq!(route_section(&Route::PodcastAutoPlaylists { id: 7 }), k);
        assert_eq!(
            route_section(&Route::PodcastConfigEdit {
                id: 7,
                config_id: 2
            }),
            k
        );
    }

    #[test]
    fn playlist_pages_fold_into_playlists_section() {
        let k = Some(NavKey::Builtin(BuiltinNav::Playlists));
        assert_eq!(route_section(&Route::Playlists {}), k);
        assert_eq!(route_section(&Route::PlaylistDetail { id: 3 }), k);
        assert_eq!(route_section(&Route::PlaylistEdit { id: 3 }), k);
    }

    #[test]
    fn settings_subpages_fold_into_settings_section() {
        let k = Some(NavKey::Builtin(BuiltinNav::Settings));
        assert_eq!(route_section(&Route::ViewConfig {}), k);
        assert_eq!(route_section(&Route::ConfigOverridesEdit {}), k);
        assert_eq!(route_section(&Route::ConfigureDock {}), k);
        // Editing your own account is reached from Settings → keeps Settings lit.
        assert_eq!(route_section(&Route::UserEdit { id: 5 }), k);
        // Device/Server logs + Polling are their OWN secondary nav destinations
        // now (each a distinct BuiltinNav key), not settings subpages — they keep
        // their own section highlighted rather than folding into Settings.
        assert_eq!(
            route_section(&Route::DeviceLogs {}),
            Some(NavKey::Builtin(BuiltinNav::DeviceLogs))
        );
    }

    #[test]
    fn chrome_and_episode_routes_have_no_section() {
        assert_eq!(route_section(&Route::Menu {}), None);
        assert_eq!(route_section(&Route::Home {}), None);
        assert_eq!(route_section(&Route::EpisodeDetail { id: 1 }), None);
        // Standalone failsafe pages — no nav section.
        assert_eq!(route_section(&Route::CacheControl {}), None);
        assert_eq!(route_section(&Route::NotFound {}), None);
    }

    #[test]
    fn episode_detail_inherits_the_previous_section() {
        // Opened from a podcast → Podcasts; opened from the queue → Queue.
        assert_eq!(
            active_nav_key(
                &Route::EpisodeDetail { id: 1 },
                Some(&Route::PodcastDetail { id: 9 }),
                &[],
            ),
            Some(NavKey::Builtin(BuiltinNav::Podcasts)),
        );
        assert_eq!(
            active_nav_key(&Route::EpisodeDetail { id: 1 }, Some(&Route::Queue {}), &[]),
            Some(NavKey::Builtin(BuiltinNav::Queue)),
        );
        // No history (deep link) → nothing to inherit.
        assert_eq!(
            active_nav_key(&Route::EpisodeDetail { id: 1 }, None, &[]),
            None
        );
    }

    #[test]
    fn non_episode_routes_ignore_previous() {
        // A normal route maps directly regardless of where you came from.
        assert_eq!(
            active_nav_key(&Route::Podcasts {}, Some(&Route::Queue {}), &[]),
            Some(NavKey::Builtin(BuiltinNav::Podcasts)),
        );
    }

    #[test]
    fn pinned_playlist_detail_highlights_its_pin() {
        // On a pinned playlist's detail, the pin lights (not the Playlists item)...
        assert_eq!(
            active_nav_key(&Route::PlaylistDetail { id: 3 }, None, &[3]),
            Some(NavKey::Pin(3)),
        );
        // ...while an unpinned playlist detail folds into Playlists.
        assert_eq!(
            active_nav_key(&Route::PlaylistDetail { id: 3 }, None, &[7]),
            Some(NavKey::Builtin(BuiltinNav::Playlists)),
        );
    }

    fn make_config(order: Vec<NavKey>, hidden: Vec<NavKey>, pins: Vec<i32>) -> ClientConfig {
        ClientConfig {
            server_url: Some("https://example.com".into()),
            server_kind: Default::default(),
            access_token: Some("token".into()),
            refresh_jwt: None,
            server_setup: true,
            is_admin: false,
            nav: NavConfig {
                order,
                hidden,
                pinned_playlists: pins,
            },
            playback_prefs: PlaybackPrefs::default(),
            font_size: Default::default(),
            device_logs: Default::default(),
            manual_offline: false,
            disabled_discover_providers: Vec::new(),
            swipe_prefs: Default::default(),
            download_prefs: Default::default(),
        }
    }

    #[test]
    fn default_order_produces_eight_items() {
        // Visible default = the 8 primary items (Queue…Settings). The 3 secondary
        // destinations (Polling, Device/Server logs) are hidden by default, so
        // they drop out of the dock/sidebar/menu until the user opts them in.
        let config = ClientConfig::default();
        let items = nav_items(&config, false);
        assert_eq!(items.len(), 8);
    }

    #[test]
    fn hidden_items_are_dropped() {
        let config = make_config(
            vec![
                NavKey::Builtin(BuiltinNav::Queue),
                NavKey::Builtin(BuiltinNav::Latest),
                NavKey::Builtin(BuiltinNav::Playlists),
            ],
            vec![NavKey::Builtin(BuiltinNav::Latest)],
            Vec::new(),
        );
        let items = nav_items(&config, false);
        assert_eq!(items.len(), 2);
        assert!(!items.iter().any(|i| i.label == "Latest"));
    }

    #[test]
    fn admin_hidden_when_not_admin() {
        let config = make_config(
            vec![
                NavKey::Builtin(BuiltinNav::Queue),
                NavKey::Builtin(BuiltinNav::Polling),
            ],
            Vec::new(),
            Vec::new(),
        );
        let items = nav_items(&config, false);
        assert_eq!(items.len(), 1);
        assert!(!items.iter().any(|i| i.admin_only));
    }

    #[test]
    fn admin_visible_when_admin() {
        let config = make_config(
            vec![
                NavKey::Builtin(BuiltinNav::Queue),
                NavKey::Builtin(BuiltinNav::Polling),
            ],
            Vec::new(),
            Vec::new(),
        );
        let items = nav_items(&config, true);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.admin_only));
    }

    #[test]
    fn pins_appended_after_builtins() {
        let config = make_config(
            vec![NavKey::Builtin(BuiltinNav::Queue)],
            Vec::new(),
            vec![1, 2],
        );
        let items = nav_items(&config, false);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[1].key, NavKey::Pin(1)));
        assert!(matches!(items[2].key, NavKey::Pin(2)));
    }

    #[test]
    fn unknown_keys_are_skipped() {
        // A Pin with an id that doesn't correspond to a known playlist
        // should still appear (placeholder label); only truly unknown
        // builtin variants would be skipped, but all are covered.
        let config = make_config(
            vec![NavKey::Builtin(BuiltinNav::Queue)],
            Vec::new(),
            vec![999],
        );
        let items = nav_items(&config, false);
        assert_eq!(items.len(), 2);
    }
}
