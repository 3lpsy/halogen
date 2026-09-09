//! The app's route table.

use dioxus::prelude::*;

// Layouts (outer → inner): a render-error boundary wrapping everything, then the
// auth guard, then the navbar/dock chrome.
use crate::app::RootErrorBoundary;
use crate::layouts::AppLayout;
use crate::root_guard::RootGuard;

// Page components — one per route.
use crate::pages::admin_users::{AdminUserCreate, AdminUserEdit, AdminUsers};
use crate::pages::auth::{EmbeddedServerSetup, Login};
use crate::pages::cache_control::CacheControl;
use crate::pages::config_overrides_form::ConfigOverridesEdit;
use crate::pages::configure_swipes::ConfigureSwipes;
use crate::pages::discover::{Discover, DiscoverDetail, DiscoverEpisode, DiscoverPodcast};
use crate::pages::dock_config::ConfigureDock;
use crate::pages::downloads::Downloads;
use crate::pages::episode::bulk_episode_playlists::BulkEpisodePlaylists;
use crate::pages::episode::episode_detail::EpisodeDetail;
use crate::pages::episode::episode_metadata::EpisodeMetadata;
use crate::pages::episode::episode_playlists::EpisodePlaylists;
use crate::pages::history::History;
use crate::pages::home::Home;
use crate::pages::latest::Latest;
use crate::pages::logs::{DeviceLogs, Logs, ServerLogs};
use crate::pages::menu::Menu;
use crate::pages::not_found::NotFound;
use crate::pages::playlists::Playlists;
use crate::pages::playlists::playlist_detail::PlaylistDetail;
use crate::pages::playlists::playlist_form::{PlaylistCreate, PlaylistEdit};
use crate::pages::playlists::playlist_reorder_by::PlaylistReorderBy;
use crate::pages::podcasts::Podcasts;
use crate::pages::podcasts::podcast_auto_playlists::PodcastAutoPlaylists;
use crate::pages::podcasts::podcast_config_form::{PodcastConfigCreate, PodcastConfigEdit};
use crate::pages::podcasts::podcast_create::PodcastCreate;
use crate::pages::podcasts::podcast_detail::PodcastDetail;
use crate::pages::podcasts::podcast_edit::PodcastEdit;
use crate::pages::podcasts::podcast_metadata::PodcastMetadata;
use crate::pages::polling::Polling;
use crate::pages::queue::Queue;
use crate::pages::server_errors::ServerErrors;
use crate::pages::settings::Settings;
use crate::pages::settings::accounts::SettingsAccounts;
use crate::pages::settings::add_embedded_user::AddEmbeddedUser;
use crate::pages::settings::downloads::SettingsDownloads;
use crate::pages::settings::playback::SettingsPlayback;
use crate::pages::settings::podcasts::SettingsPodcasts;
use crate::pages::settings::server::SettingsServer;
use crate::pages::settings::ui::SettingsUi;
use crate::pages::user_edit::UserEdit;
use crate::pages::view_config::ViewConfig;

#[derive(Debug, Clone, Routable, PartialEq)]
pub enum Route {
    // Outermost layout: a render-error boundary that wraps EVERY route (including
    // the standalone failsafe pages below), so a render-time panic anywhere lands
    // on the in-router recovery page (`AppError`) — see `crate::app::RootErrorBoundary`.
    #[layout(RootErrorBoundary)]
    // Standalone recovery pages — under the error boundary but with no auth guard
    // and no navbar chrome (they render even signed-out / mid-failure).
    #[route("/cache-control")]
    CacheControl {},

    // 404 escape hatch. The catch-all redirect funnels every otherwise-unmatched path (an unknown route, or a typed
    // param like `/episodes/abc` that fails to parse) to the canonical `/not-found`. The catch-all is least-specific,
    // so real routes still win. The closure param must be named `segments` to match the `:..segments` catch-all (the
    // macro matches by exact name); `let _` keeps it from warning as unused.
    #[redirect("/:..segments", |segments: Vec<String>| { let _ = segments; Route::NotFound {} })]
    #[route("/not-found")]
    NotFound {},

    // Everything below is wrapped by RootGuard, which redirects unauthenticated
    // users to the login page (server URL + credentials on one form, like the
    // native apps). Auth routes sit directly under it (no app chrome); app
    // routes nest in AppLayout.
    #[layout(RootGuard)]
    // Legacy path from the old two-step flow — old bookmarks land on Login.
    #[redirect("/auth/server-setup", || Route::Login {})]
    #[route("/auth/login")]
    Login {},

    // Embedded Server confirmation/reconnect (native builds; the entry link is
    // gated on availability). An auth route like the two above — RootGuard
    // treats it as such.
    #[route("/auth/embedded-setup")]
    EmbeddedServerSetup {},

    #[layout(AppLayout)]
    #[route("/")]
    Home {},

    #[route("/queue")]
    Queue {},

    #[route("/latest")]
    Latest {},

    #[route("/podcasts")]
    Podcasts {},

    // Add a podcast by feed URL. Static `create` before the dynamic `/podcasts/:id`
    // so "create" is never parsed as an i32 (same as `/playlists/create`).
    #[route("/podcasts/create")]
    PodcastCreate {},

    // Detail pages (dynamic segment).
    #[route("/episodes/:id")]
    EpisodeDetail { id: i32 },

    // Multiselect playlist picker for an episode. Static `playlists` segment after
    // the dynamic `:episode_id` — a deeper path than `/episodes/:id`, no ambiguity.
    #[route("/episodes/:episode_id/playlists")]
    EpisodePlaylists { episode_id: i32 },

    // Read-only episode metadata. Static `metadata` leaf on the dynamic id —
    // deeper than `/episodes/:id`, no ambiguity.
    #[route("/episodes/:id/metadata")]
    EpisodeMetadata { id: i32 },

    // Bulk "add to playlist" picker for a multiselect — carries the selected episode
    // ids comma-joined in the path. Static `bulk/playlists` segments, distinct depth
    // from `/episodes/:id` and `/episodes/:episode_id/playlists`, so no ambiguity.
    #[route("/episodes/bulk/playlists/:ids")]
    BulkEpisodePlaylists { ids: String },

    #[route("/podcasts/:id")]
    PodcastDetail { id: i32 },

    // Read-only podcast metadata. Static `metadata` leaf on the dynamic id —
    // deeper than `/podcasts/:id`, no ambiguity.
    #[route("/podcasts/:id/metadata")]
    PodcastMetadata { id: i32 },

    // Edit the podcast's own fields (title / description / feed URL). Static
    // `edit` leaf, like the config routes below.
    #[route("/podcasts/:id/edit")]
    PodcastEdit { id: i32 },

    // Manage a podcast's download/poll config. Static `create` before the dynamic
    // `:config_id` (same reasoning as `/playlists/create`).
    #[route("/podcasts/:id/config/create")]
    PodcastConfigCreate { id: i32 },

    #[route("/podcasts/:id/config/:config_id/edit")]
    PodcastConfigEdit { id: i32, config_id: i32 },

    // Configure which playlists a podcast auto-adds new episodes to. Static
    // `auto-playlists` segment, like the config routes — never parsed as an i32.
    #[route("/podcasts/:id/auto-playlists")]
    PodcastAutoPlaylists { id: i32 },

    #[route("/playlists")]
    Playlists {},

    // Static segment before the dynamic `:id` so "/playlists/create" never tries
    // to parse "create" as an i32.
    #[route("/playlists/create")]
    PlaylistCreate {},

    #[route("/playlists/:id")]
    PlaylistDetail { id: i32 },

    #[route("/playlists/:id/edit")]
    PlaylistEdit { id: i32 },

    // Smart-reorder a playlist's episodes by a chosen field + direction.
    #[route("/playlists/:id/reorder-by")]
    PlaylistReorderBy { id: i32 },

    #[route("/downloads")]
    Downloads {},

    // Online-only podcast discovery. The detail `:id` is the provider-scoped
    // synthetic string id from the search result (NOT an i32 like other details).
    // Static `/discover` before the dynamic sibling, like `/podcasts`.
    #[route("/discover")]
    Discover {},

    #[route("/discover/podcasts/:id")]
    DiscoverPodcast { id: String },

    #[route("/discover/episodes/:id")]
    DiscoverEpisode { id: String },

    #[route("/discover/:id")]
    DiscoverDetail { id: String },

    #[route("/history")]
    History {},

    #[route("/settings")]
    Settings {},

    // Settings sub-pages — one per group on the `/settings` menu page. All
    // static segments, so none shadows another `/settings/*` route.
    #[route("/settings/playback")]
    SettingsPlayback {},

    #[route("/settings/downloads")]
    SettingsDownloads {},

    #[route("/settings/ui")]
    SettingsUi {},

    #[route("/settings/accounts")]
    SettingsAccounts {},

    // Create + switch to a new user on the EMBEDDED server (username only —
    // the app manages passwords). The Add-account buttons route here instead
    // of Login when the active account is embedded.
    #[route("/settings/accounts/add-embedded")]
    AddEmbeddedUser {},

    #[route("/settings/server")]
    SettingsServer {},

    // Admin-only (OPML import/export).
    #[route("/settings/podcasts")]
    SettingsPodcasts {},

    // Admin-only, reached from the Settings page.
    #[route("/settings/config")]
    ViewConfig {},

    // Admin-only config-overrides editor, reached from the View Config header.
    #[route("/settings/config/overrides")]
    ConfigOverridesEdit {},

    // Configure the dock/nav order + visibility (frontend-only).
    #[route("/settings/dock")]
    ConfigureDock {},

    // Configure per-page episode swipe actions (frontend-only).
    #[route("/settings/configure-swipes")]
    ConfigureSwipes {},

    // Edit your own account (username + password). Reached from Settings → Accounts;
    // the page itself guards that `id` is the active user.
    #[route("/user/:id/edit")]
    UserEdit { id: i32 },

    // Mobile-only full navigation page, opened from the dock's "More" slot.
    #[route("/menu")]
    Menu {},

    #[route("/polling")]
    Polling {},

    #[route("/logs")]
    Logs {},

    #[route("/logs/device")]
    DeviceLogs {},

    #[route("/admin/logs")]
    ServerLogs {},

    // Admin-only view of the server's persisted failure histories (RSS sync +
    // episode downloads), reached from Settings → Server.
    #[route("/admin/errors")]
    ServerErrors {},

    // Admin-only user management, reached from Settings → Accounts → "Manage".
    #[route("/admin/users")]
    AdminUsers {},

    // Create a new server user (admin-only). Static `create` before the dynamic
    // `:id` sibling so "create" is never parsed as an i32 (same reasoning as
    // `/playlists/create`).
    #[route("/admin/users/create")]
    AdminUserCreate {},

    // Edit another user (admin-only). Dynamic id with a static `edit` leaf, deeper
    // than `/admin/users`, so no ambiguity.
    #[route("/admin/users/:id/edit")]
    AdminUserEdit { id: i32 },
}

#[cfg(test)]
mod route_parse_tests {
    //! Test parsing lazy detail routes with shared static prefixes natively. A parser loop then hits nextest's timeout
    //! instead of freezing the browser before playlists/podcasts can mount.
    use super::*;
    use std::str::FromStr;

    fn parses(s: &str) -> Route {
        Route::from_str(s).unwrap_or_else(|e| panic!("`{s}` failed to parse: {e}"))
    }

    #[test]
    fn detail_route_with_static_sibling_parses_playlists() {
        assert_eq!(parses("/playlists/2"), Route::PlaylistDetail { id: 2 });
    }

    #[test]
    fn detail_route_with_static_sibling_parses_podcasts() {
        assert_eq!(parses("/podcasts/1"), Route::PodcastDetail { id: 1 });
    }

    #[test]
    fn detail_route_without_sibling_parses_episodes() {
        assert_eq!(parses("/episodes/8"), Route::EpisodeDetail { id: 8 });
    }

    #[test]
    fn nested_podcast_config_routes_parse() {
        // The static `create` leaf and the dynamic `:config_id/edit` sibling don't
        // collide, and neither shadows `/podcasts/:id`.
        assert_eq!(
            parses("/podcasts/3/config/create"),
            Route::PodcastConfigCreate { id: 3 }
        );
        assert_eq!(
            parses("/podcasts/3/config/7/edit"),
            Route::PodcastConfigEdit {
                id: 3,
                config_id: 7
            }
        );
        assert_eq!(
            parses("/podcasts/3/auto-playlists"),
            Route::PodcastAutoPlaylists { id: 3 }
        );
        assert_eq!(parses("/podcasts/3"), Route::PodcastDetail { id: 3 });
        // Static `create` leaf must win over the dynamic `:id`.
        assert_eq!(parses("/podcasts/create"), Route::PodcastCreate {});
    }

    #[test]
    fn metadata_and_edit_routes_parse() {
        // Static `metadata` / `edit` leaves on the dynamic ids — deeper than the
        // detail routes, so neither shadows `/podcasts/:id` / `/episodes/:id`.
        assert_eq!(
            parses("/podcasts/3/metadata"),
            Route::PodcastMetadata { id: 3 }
        );
        assert_eq!(parses("/podcasts/3/edit"), Route::PodcastEdit { id: 3 });
        assert_eq!(
            parses("/episodes/8/metadata"),
            Route::EpisodeMetadata { id: 8 }
        );
        // The detail routes are unaffected.
        assert_eq!(parses("/podcasts/3"), Route::PodcastDetail { id: 3 });
        assert_eq!(parses("/episodes/8"), Route::EpisodeDetail { id: 8 });
    }

    #[test]
    fn settings_subpage_routes_parse() {
        // All-static `/settings/*` siblings — none shadows `/settings` itself or
        // the pre-existing config/dock/swipes routes.
        assert_eq!(parses("/settings"), Route::Settings {});
        assert_eq!(parses("/settings/playback"), Route::SettingsPlayback {});
        assert_eq!(parses("/settings/downloads"), Route::SettingsDownloads {});
        assert_eq!(parses("/settings/ui"), Route::SettingsUi {});
        assert_eq!(parses("/settings/accounts"), Route::SettingsAccounts {});
        assert_eq!(parses("/settings/server"), Route::SettingsServer {});
        assert_eq!(parses("/settings/podcasts"), Route::SettingsPodcasts {});
        // Pre-existing settings routes still win their own paths.
        assert_eq!(parses("/settings/config"), Route::ViewConfig {});
        assert_eq!(parses("/settings/dock"), Route::ConfigureDock {});
        assert_eq!(
            parses("/settings/configure-swipes"),
            Route::ConfigureSwipes {}
        );
    }

    #[test]
    fn discover_routes_parse_with_string_id() {
        // `/discover` is a static prefix of the dynamic `/discover/:id`, and the
        // id is a String (synthetic hash), not an i32.
        assert_eq!(parses("/discover"), Route::Discover {});
        assert_eq!(
            parses("/discover/podcasts/show"),
            Route::DiscoverPodcast { id: "show".into() }
        );
        assert_eq!(
            parses("/discover/episodes/episode"),
            Route::DiscoverEpisode {
                id: "episode".into()
            }
        );
        assert_eq!(
            parses("/discover/itunes-0123abcd"),
            Route::DiscoverDetail {
                id: "itunes-0123abcd".to_string()
            }
        );
    }

    #[test]
    fn static_routes_that_extend_a_sibling_parse() {
        assert_eq!(parses("/auth/login"), Route::Login {});
        // The old two-step flow's first page redirects to the merged Login.
        assert_eq!(parses("/auth/server-setup"), Route::Login {});
        assert_eq!(parses("/settings/config"), Route::ViewConfig {});
        assert_eq!(parses("/admin/logs"), Route::ServerLogs {});
        assert_eq!(parses("/admin/errors"), Route::ServerErrors {});
        assert_eq!(parses("/admin/users"), Route::AdminUsers {});
        assert_eq!(parses("/logs/device"), Route::DeviceLogs {});
        assert_eq!(parses("/playlists"), Route::Playlists {});
        assert_eq!(parses("/podcasts"), Route::Podcasts {});
    }

    #[test]
    fn standalone_and_user_edit_routes_parse() {
        // The standalone recovery pages (only the error-boundary layout, no guard).
        assert_eq!(parses("/cache-control"), Route::CacheControl {});
        assert_eq!(parses("/not-found"), Route::NotFound {});
        // `/user/:id/edit`: a dynamic id with a static `edit` leaf, no sibling to
        // shadow it.
        assert_eq!(parses("/user/5/edit"), Route::UserEdit { id: 5 });
        // `/admin/users/:id/edit`: dynamic id, static `edit` leaf, no sibling to
        // shadow it (and `/admin/users` stays the static list route).
        assert_eq!(
            parses("/admin/users/7/edit"),
            Route::AdminUserEdit { id: 7 }
        );
        // The static `create` leaf must win over the dynamic `:id` sibling, and the
        // list route stays intact.
        assert_eq!(parses("/admin/users/create"), Route::AdminUserCreate {});
        assert_eq!(parses("/admin/users"), Route::AdminUsers {});
    }

    /// The catch-all redirect is the bad-link escape hatch: any path that matches
    /// no real route resolves to `NotFound`. Real routes (more specific) still win.
    #[test]
    fn unknown_paths_redirect_to_not_found() {
        assert_eq!(parses("/totally-bogus"), Route::NotFound {});
        assert_eq!(parses("/no/such/page"), Route::NotFound {});
        // A real route is unaffected — the catch-all never shadows it.
        assert_eq!(parses("/queue"), Route::Queue {});
    }

    // Minimal repro isolated from the app's enum: a static route that is a strict
    // prefix of a dynamic sibling — the exact shape of /podcasts + /podcasts/:id.
    mod minimal {
        use dioxus::prelude::*;
        use std::str::FromStr;

        #[component]
        fn L() -> Element {
            rsx! {}
        }
        #[component]
        fn D(id: i32) -> Element {
            rsx! {}
        }

        #[derive(Debug, Clone, Routable, PartialEq)]
        enum T {
            #[route("/podcasts")]
            L {},
            #[route("/podcasts/:id")]
            D { id: i32 },
        }

        #[test]
        fn prefix_sibling_dynamic_route_parses() {
            assert_eq!(
                T::from_str("/podcasts/1").expect("parse /podcasts/1"),
                T::D { id: 1 }
            );
        }
    }
}
