//! Admin-only "view config" page (`/settings/config`). Fetches the server's reconciled runtime config (`GET
//! /api/v1/config`, secrets already stripped server-side) and renders it read-only, grouped into labelled sections with
//! an "Other" catch-all at the bottom for ungroupable knobs.

use dioxus::prelude::*;
use halogen_wire::ConfigData;

use crate::components::{BackButton, ConfirmModal, resource_view};
use halogen_webui_component_icons::{ArrowPath, Pencil};
use halogen_webui_hooks::{
    use_config, use_confirm_action, use_is_admin, use_is_offline, use_toast,
};

/// One labelled key/value row in a section.
struct Row {
    label: &'static str,
    value: String,
}

/// A titled group of rows.
struct Section {
    title: &'static str,
    rows: Vec<Row>,
}

fn opt(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "Not set".to_string())
}

fn yn(v: bool) -> String {
    if v { "Yes" } else { "No" }.to_string()
}

fn list(v: &[String]) -> String {
    if v.is_empty() {
        "None (permissive)".to_string()
    } else {
        v.join(", ")
    }
}

/// Partition the flat [`ConfigData`] into display sections. Every field appears
/// exactly once; ungroupable knobs land in the trailing "Other" section.
fn sections(c: &ConfigData) -> Vec<Section> {
    vec![
        Section {
            title: "Server",
            rows: vec![
                Row {
                    label: "Listen address",
                    value: c.listen_address.clone(),
                },
                Row {
                    label: "Listen port",
                    value: c.listen_port.to_string(),
                },
                Row {
                    label: "Polling service disabled",
                    value: yn(c.server_disable_polling_service),
                },
                Row {
                    label: "CORS allowed origins",
                    value: list(&c.cors_allowed_origins),
                },
                Row {
                    label: "Public server enabled",
                    value: yn(c.enable_public_server),
                },
                Row {
                    label: "Public root",
                    value: opt(&c.public_root),
                },
                Row {
                    label: "Public URL path",
                    value: c.public_url_path.clone(),
                },
            ],
        },
        Section {
            title: "Database",
            rows: vec![
                Row {
                    label: "Path",
                    value: c.db_path.clone(),
                },
                Row {
                    label: "Skip migrations",
                    value: yn(c.db_no_migrate),
                },
                Row {
                    label: "Skip default playlist",
                    value: yn(c.db_skip_default_playlist),
                },
            ],
        },
        Section {
            title: "Media",
            rows: vec![Row {
                label: "Root",
                value: c.media_root.clone(),
            }],
        },
        Section {
            title: "Subscriptions & Polling",
            rows: vec![
                Row {
                    label: "Fallback poll interval (s)",
                    value: c.subscription_fallback_poll_interval_secs.to_string(),
                },
                Row {
                    label: "Poll wake interval (s)",
                    value: c.subscription_poll_wake_interval_secs.to_string(),
                },
                Row {
                    label: "Fallback max episodes",
                    value: c.subscription_fallback_max_episodes.to_string(),
                },
                Row {
                    label: "Max concurrent downloads",
                    value: c.subscription_max_concurrent_downloads.to_string(),
                },
                Row {
                    label: "Max concurrent polls",
                    value: c.subscription_max_poll_concurrent.to_string(),
                },
                Row {
                    label: "Auto-download new episodes",
                    value: yn(c.subscription_poll_auto_download_enabled),
                },
                Row {
                    label: "Auto-add to start of playlists",
                    value: yn(c.subscription_auto_playlist_add_to_start),
                },
                Row {
                    label: "No sync before",
                    value: c.subscription_no_sync_before.clone(),
                },
                Row {
                    label: "Sync on start",
                    value: yn(c.subscription_sync_on_start),
                },
            ],
        },
        Section {
            title: "Auth",
            rows: vec![Row {
                label: "Token expiry (minutes)",
                value: c.auth_token_expiry_minutes.to_string(),
            }],
        },
        Section {
            title: "Episodes",
            rows: vec![Row {
                label: "Playback complete (last %)",
                value: c.episode_playback_complete_percentage.to_string(),
            }],
        },
        Section {
            title: "Logging",
            rows: vec![
                Row {
                    label: "Level",
                    value: c.log_level.clone(),
                },
                Row {
                    label: "File",
                    value: opt(&c.log_file),
                },
                Row {
                    label: "Show target",
                    value: yn(c.log_target),
                },
                Row {
                    label: "Show file name",
                    value: yn(c.log_file_name),
                },
                Row {
                    label: "Show line number",
                    value: yn(c.log_line_number),
                },
            ],
        },
        Section {
            title: "Admin",
            rows: vec![
                Row {
                    label: "Username",
                    value: opt(&c.admin_username),
                },
                Row {
                    label: "Seeding disabled",
                    value: yn(c.admin_disable_seed),
                },
            ],
        },
        Section {
            title: "Other",
            rows: vec![
                Row {
                    label: "OPML file",
                    value: opt(&c.opml_file),
                },
                Row {
                    label: "Mock downloads (dev)",
                    value: yn(c.dev_use_mock_download),
                },
                Row {
                    label: "Seed data (dev)",
                    value: yn(c.dev_seed_data),
                },
            ],
        },
    ]
}

#[component]
pub fn ViewConfig() -> Element {
    let cfg = use_config();
    let nav = use_navigator();
    let is_admin = use_is_admin();
    let toast = use_toast();

    // The edit + restart actions are admin-only and need the server reachable
    // (this is an online-only surface). When either fails, both are disabled.
    let is_offline = use_is_offline()();
    let actions_disabled = !is_admin() || is_offline;

    // Restart confirmation flow.
    let restart = use_confirm_action();

    // Fetch the reconciled server config once (re-runs if the client config —
    // server URL / token — changes). Secrets are already stripped server-side.
    let config = use_resource(move || {
        // `api_client()` is None when unconfigured OR manually offline → "Go Offline"
        // suppresses this fetch.
        let client = cfg.read().api_client_or_err();
        async move {
            let client = client?;
            client.get_config().await.map_err(|e| e.to_string())
        }
    });

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row with the override-editor + restart actions on
            // the right (admin + online only) — stays put while the config scrolls.
            div { class: "p-2 flex items-center justify-between gap-2",
                BackButton {}
                div { class: "flex items-center gap-1",
                    button {
                        "aria-label": "Edit config overrides",
                        class: "btn btn-square btn-ghost",
                        disabled: actions_disabled,
                        onclick: move |_| {
                            nav.push("/settings/config/overrides");
                        },
                        Pencil { class: "w-5 h-5" }
                    }
                    button {
                        "aria-label": "Restart server",
                        class: "btn btn-square btn-ghost",
                        disabled: actions_disabled,
                        onclick: move |_| restart.open(),
                        ArrowPath { class: "w-5 h-5" }
                    }
                }
            }
            // Scrollable config beneath the pinned back row.
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-8",
            h1 { class: "text-3xl font-bold mb-2", "Server Configuration" }
            p {
                class: "text-muted mb-4",
                "The reconciled runtime configuration in effect on the server. Secrets (token secret, admin password) are not shown."
            }

            {
                resource_view(&*config.read_unchecked(), "config", |data| {
                    let groups = sections(data);
                    rsx! {
                        for section in groups {
                            div {
                                class: "space-y-2 p-4 bg-base-200 rounded-lg",
                                h2 {
                                    class: "text-xl font-semibold mb-2",
                                    "{section.title}"
                                }
                                for row in section.rows {
                                    div {
                                        class: "flex items-start justify-between gap-4 py-1 border-b border-base-300 last:border-0",
                                        span { class: "text-muted", "{row.label}" }
                                        span {
                                            class: "font-mono text-right break-all",
                                            "{row.value}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                })
            }
            }

            // Restart confirmation — re-exec applies any saved overrides.
            if (restart.open)() {
                ConfirmModal {
                    title: "Restart the server?",
                    body: "The server will re-exec to apply any saved config overrides. The connection will drop briefly while it restarts.",
                    confirm_label: "Restart",
                    busy: (restart.busy)(),
                    title_id: "confirm-restart-title",
                    on_cancel: move |_| restart.close(),
                    on_confirm: move |_| {
                        restart.run(
                            cfg,
                            |c| async move { c.restart_server().await },
                            move |res| match res {
                                Ok(()) => {
                                    toast.success("Server restarting… reconnect in a moment.")
                                }
                                Err(e) => toast.error(format!("Restart failed: {e}")),
                            },
                        );
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ConfigData {
        ConfigData {
            listen_address: "0.0.0.0".to_string(),
            listen_port: 8080,
            server_disable_polling_service: false,
            db_path: "/data/halogen.db".to_string(),
            db_no_migrate: false,
            db_skip_default_playlist: false,
            media_root: "/data/media".to_string(),
            cors_allowed_origins: vec![],
            enable_public_server: true,
            public_root: Some("/dist".to_string()),
            public_url_path: "/".to_string(),
            subscription_fallback_poll_interval_secs: 3600,
            subscription_poll_wake_interval_secs: 60,
            subscription_fallback_max_episodes: 50,
            subscription_max_concurrent_downloads: 4,
            subscription_max_poll_concurrent: 8,
            subscription_poll_auto_download_enabled: false,
            subscription_auto_playlist_add_to_start: false,
            subscription_no_sync_before: "2026-01-01".to_string(),
            subscription_sync_on_start: false,
            auth_token_expiry_minutes: 43200,
            episode_playback_complete_percentage: 4,
            log_file: None,
            log_level: "info".to_string(),
            log_target: false,
            log_file_name: true,
            log_line_number: true,
            admin_username: Some("admin".to_string()),
            admin_disable_seed: false,
            opml_file: None,
            dev_use_mock_download: false,
            dev_seed_data: false,
            overridden_fields: vec![],
            config_overrides_disabled: false,
            config_overrides_path: None,
            config_overrides_loaded: false,
        }
    }

    /// Every field is surfaced exactly once across all sections (32 fields), and
    /// the trailing section is the "Other" catch-all.
    #[test]
    fn sections_cover_all_fields_with_other_last() {
        let groups = sections(&sample());
        let total_rows: usize = groups.iter().map(|s| s.rows.len()).sum();
        assert_eq!(
            total_rows, 32,
            "every ConfigData field should map to one row"
        );
        assert_eq!(groups.last().unwrap().title, "Other");
    }

    /// Formatting helpers render the empty / unset / bool cases as expected.
    #[test]
    fn formatters_render_human_values() {
        assert_eq!(opt(&None), "Not set");
        assert_eq!(opt(&Some("x".to_string())), "x");
        assert_eq!(yn(true), "Yes");
        assert_eq!(yn(false), "No");
        assert_eq!(list(&[]), "None (permissive)");
        assert_eq!(list(&["a".to_string(), "b".to_string()]), "a, b");
    }
}
