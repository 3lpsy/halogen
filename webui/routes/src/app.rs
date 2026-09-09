//! The launch root (`App`) and the cross-platform root error boundary.

use dioxus::prelude::*;

use crate::Route;
use crate::components::ToastContainer;
use crate::pages::error::{AppError, FatalError};
use halogen_webui_provider_app::AppProviders;

#[component]
pub fn App() -> Element {
    rsx! {
        // Static head metadata lives in app/index.html for pre-hydration consumers; Tailwind remains a Dioxus web
        // resource. This outer boundary catches provider render failures with a context-free FatalError fallback. Route
        // failures use the inner boundary's richer recovery.
        ErrorBoundary {
            handle_error: |errors: ErrorContext| {
                halogen_webui_logging::error!(
                    "Fatal error at/above the providers — app cannot continue: {errors:?}"
                );
                rsx! { FatalError { errors } }
            },
            AppProviders {
                // Global toast overlay — rendered once inside the providers, above all
                // routes (including auth screens, which sit outside AppLayout).
                ToastContainer {}
                Router::<Route> {}
            }
        }
    }
}

/// Catch route render errors inside Router and recover through navigation plus clear_errors, preserving the native
/// webview bridge. This excludes event/task/worker panics and provider initialization above Router.
#[component]
pub fn RootErrorBoundary() -> Element {
    rsx! {
        ErrorBoundary {
            handle_error: |errors: ErrorContext| {
                halogen_webui_logging::error!(
                    "Unhandled UI render error caught by the root boundary: {errors:?}"
                );
                rsx! { AppError { errors } }
            },
            Outlet::<Route> {}
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod detail_route_render_tests {
    //! Drive the full authenticated app at lazy detail routes in an isolated native VirtualDom and bound render passes.
    //! Parsing is tested separately; this detects self-dirtying render loops that freeze browser e2e before DOM flush.
    use super::*;
    use dioxus::history::{History, MemoryHistory};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    #[component]
    fn Shell(path: String) -> Element {
        let p = path.clone();
        use_hook(move || {
            provide_context(Rc::new(MemoryHistory::with_initial_path(p)) as Rc<dyn History>)
        });
        rsx! {
            AppProviders {
                Router::<Route> {}
            }
        }
    }

    /// Point the native config/data dirs (both resolved via `directories:: ProjectDirs`, which honors `XDG_*` on Linux)
    /// at a per-test sandbox with an authed `client.json`, so RootGuard doesn't bounce to the login page and the
    /// worker's store opens inside the sandbox. nextest runs each test in its own process, so the env mutation can't
    /// race another test.
    fn sandbox_dirs(tag: &str) {
        let base =
            std::env::temp_dir().join(format!("halogen-route-render-{tag}-{}", std::process::id()));
        let cfg = base.join("config");
        let data = base.join("data");
        // Self-cleaning: wipe any previous run's sandbox so temp dirs don't pile up.
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(cfg.join("halogen")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            cfg.join("halogen/client.json"),
            // Closed port: the worker's pull fails fast and goes Offline.
            r#"{"server_url":"http://127.0.0.1:9","access_token":"test-token","refresh_jwt":null,"server_setup":true,"is_admin":false}"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &cfg);
            std::env::set_var("XDG_DATA_HOME", &data);
        }
    }

    /// Embedded-mode sandbox: an authed EMBEDDED account (registry + its `e{id}` config), so /downloads runs
    /// `ListSource::ServerDownloads`. No URL resolver is installed in this renderless build, so the persisted
    /// closed-port URL stands and the worker goes Offline fast (like the remote sandbox above).
    fn sandbox_dirs_embedded(tag: &str) {
        let base =
            std::env::temp_dir().join(format!("halogen-route-render-{tag}-{}", std::process::id()));
        let cfg = base.join("config");
        let data = base.join("data");
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(cfg.join("halogen/e7")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            cfg.join("halogen/accounts.json"),
            r#"{"active_user_id":7,"active_kind":"Embedded","users":[{"id":7,"username":"local","needs_reauth":false,"kind":"Embedded"}]}"#,
        )
        .unwrap();
        std::fs::write(
            cfg.join("halogen/e7/client.json"),
            r#"{"server_url":"http://127.0.0.1:9","server_kind":"Embedded","access_token":"test-token","refresh_jwt":null,"server_setup":true,"is_admin":true}"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &cfg);
            std::env::set_var("XDG_DATA_HOME", &data);
        }
    }

    /// Pump the vdom until it goes quiet (no work for 250ms) or the pass budget
    /// blows. Background ticks (player position etc.) stay well under the
    /// budget in 3s; a render loop exhausts it immediately.
    async fn pump(path: &'static str) -> u32 {
        let mut vdom = VirtualDom::new_with_props(
            Shell,
            ShellProps {
                path: path.to_string(),
            },
        );
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
        let start = Instant::now();
        let mut passes = 0u32;
        while start.elapsed() < Duration::from_secs(3) {
            match tokio::time::timeout(Duration::from_millis(250), vdom.wait_for_work()).await {
                Ok(()) => {
                    vdom.render_immediate(&mut dioxus::core::NoOpMutations);
                    passes += 1;
                    if passes > 400 {
                        break;
                    }
                }
                Err(_) => break, // settled
            }
        }
        passes
    }

    // Tip for the next render-loop hunt: dioxus-core/signals emit TRACE events in debug builds, `mark_dirty` logs each
    // dirtied context's creation site and the props-memoize write guard logs the writing line. A throwaway test that
    // installs `tracing_subscriber::fmt().with_env_filter("dioxus_core=trace, dioxus_signals=trace")` around `pump()`
    // names the cycle directly.

    /// Control: same EpisodeList machinery, page does not subscribe to EpisodeState.
    #[tokio::test]
    async fn queue_route_settles() {
        sandbox_dirs("queue");
        let passes = pump("/queue").await;
        assert!(
            passes <= 400,
            "/queue never settled ({passes} render passes) — re-render loop"
        );
    }

    #[tokio::test]
    async fn playlist_detail_route_settles() {
        sandbox_dirs("playlist");
        let passes = pump("/playlists/2").await;
        assert!(
            passes <= 400,
            "/playlists/2 never settled ({passes} render passes) — re-render loop"
        );
    }

    #[tokio::test]
    async fn podcast_detail_route_settles() {
        sandbox_dirs("podcast");
        let passes = pump("/podcasts/1").await;
        assert!(
            passes <= 400,
            "/podcasts/1 never settled ({passes} render passes) — re-render loop"
        );
    }

    // Full-route settle battery: every routable page boots and goes quiet.
    // (One test per route so a loop names its page; `settle!` keeps it terse.)
    macro_rules! settle {
        ($name:ident, $tag:literal, $path:literal) => {
            #[tokio::test]
            async fn $name() {
                sandbox_dirs($tag);
                let passes = pump($path).await;
                assert!(
                    passes <= 400,
                    concat!($path, " never settled ({} render passes) — re-render loop"),
                    passes
                );
            }
        };
    }

    settle!(home_route_settles, "home", "/"); // includes the nav.replace(Queue) redirect
    settle!(latest_route_settles, "latest", "/latest");
    settle!(podcasts_route_settles, "podcasts", "/podcasts");
    settle!(
        podcast_create_route_settles,
        "podcast_create",
        "/podcasts/create"
    );
    settle!(episode_detail_route_settles, "episode", "/episodes/8"); // deep-link on-miss fetch
    settle!(playlists_route_settles, "playlists", "/playlists");
    settle!(downloads_route_settles, "downloads", "/downloads");

    /// Embedded-mode Downloads (`ListSource::ServerDownloads`) settles.
    /// Regression: the id-list snapshot gates missed the new variant — the
    /// first real desktop run panicked on /downloads with "downloads/history
    /// branch snapshots app_state".
    #[tokio::test]
    async fn downloads_route_settles_embedded() {
        sandbox_dirs_embedded("downloads-embedded");
        let passes = pump("/downloads").await;
        assert!(
            passes <= 400,
            "/downloads (embedded) never settled ({passes} render passes) — re-render loop"
        );
    }
    settle!(discover_route_settles, "discover", "/discover");
    settle!(
        discover_detail_route_settles,
        "discover_detail",
        "/discover/itunes-0123abcd"
    ); // empty-store fallback
    settle!(history_route_settles, "history", "/history");
    settle!(settings_route_settles, "settings", "/settings");
}
