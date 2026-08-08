//! Settings → Server (`/settings/server`): server identity (type/URL, the
//! embedded server's live status + data location), the admin database
//! export/import (the server↔server / embedded↔server migration and
//! full-backup story), and the admin-only server surfaces (runtime config,
//! polling history, server logs).

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::Route;
use crate::components::SettingsRow;
use halogen_ui_platform::time::sleep_ms;
use halogen_ui_state::embedded::{self, EmbeddedState};
use halogen_ui_state::hooks::{use_config, use_is_admin, use_toast};

#[component]
pub fn SettingsServer() -> Element {
    let config = use_config();
    let nav = use_navigator();
    let is_admin = use_is_admin();
    let toast = use_toast();
    let embedded_mode = config().server_kind.is_embedded();

    // Live embedded status: the supervisor's state changes outside the render
    // cycle (Starting → Running, restarts), so a one-shot read can freeze on a
    // boot-window snapshot. A page-scoped 1s poll keeps the row honest.
    let mut live_state = use_signal(embedded::state);
    use_future(move || async move {
        if !embedded::available() {
            return;
        }
        loop {
            let mut now = embedded::state();
            // The wire outranks the statics: if the supervisor claims the
            // server is down but the configured loopback URL answers
            // /healthz, it IS running — dev hot-patching (`dx serve`) can
            // hand page code fresh process statics while the previously
            // booted server keeps serving.
            if matches!(now, EmbeddedState::Stopped | EmbeddedState::Failed { .. })
                && config.peek().server_kind.is_embedded()
            {
                let url = config.peek().server_url.clone();
                if let Some(url) = url
                    && let Some(client) = halogen_ui_config::api_client_from(Some(&url), None)
                    && client.health().await.is_ok()
                {
                    let port = url
                        .rsplit(':')
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or_default();
                    now = EmbeddedState::Running { port };
                }
            }
            // Only write on an actual change — an unconditional set would
            // re-render the row every second in steady state.
            if now != *live_state.peek() {
                live_state.set(now);
            }
            sleep_ms(1000).await;
        }
    });

    // Import staging: a picked file waits for an explicit confirm (a merge is
    // safe, but appending a whole library shouldn't be one mis-tap away).
    let mut staged_import = use_signal(|| None::<(String, Vec<u8>)>);
    let transfer_busy = use_signal(|| false);

    let export_db = move |_| {
        if transfer_busy() {
            return;
        }
        let mut transfer_busy = transfer_busy;
        transfer_busy.set(true);
        spawn(async move {
            let outcome = async {
                let client = config.peek().api_client_or_err()?;
                let bytes = client
                    .export_db()
                    .await
                    .map_err(|e| format!("Export failed: {e}"))?;
                // `download_bytes` distinguishes a real write failure from the
                // browser-owned handoff — an unwritable data dir must NOT toast
                // success while the user believes they have a backup.
                halogen_ui_logging::download_bytes(
                    "halogen-export.db.gz",
                    &bytes,
                    "application/gzip",
                )
            }
            .await;
            match outcome {
                // Native returns the written path; web triggers a browser download.
                Ok(Some(path)) => toast.success(format!("Exported database to {path}")),
                Ok(None) => toast.success("Exported database"),
                Err(e) => toast.error(e),
            }
            transfer_busy.set(false);
        });
    };

    let run_import = move |_| {
        let Some((_name, bytes)) = staged_import.peek().clone() else {
            return;
        };
        if transfer_busy() {
            return;
        }
        let mut transfer_busy = transfer_busy;
        let mut staged_import = staged_import;
        transfer_busy.set(true);
        spawn(async move {
            let outcome = async {
                let client = config.peek().api_client_or_err()?;
                client
                    .import_db(bytes)
                    .await
                    .map_err(|e| format!("Import failed: {e}"))
            }
            .await;
            match outcome {
                Ok(summary) => {
                    // Users the import created got random passwords; on an
                    // embedded server, re-key them into the silent-login
                    // secrets so switching to them just works.
                    if config.peek().server_kind.is_embedded()
                        && !summary.created_usernames.is_empty()
                    {
                        halogen_ui_state::embedded_session::align_imported_users(
                            &summary.created_usernames,
                        )
                        .await;
                    }
                    toast.success(format!(
                        "Import merged: {} user(s) matched, {} created; +{} podcast(s), \
                         +{} episode(s), +{} playlist(s)",
                        summary.users_merged,
                        summary.users_created,
                        summary.podcasts_created,
                        summary.episodes_created,
                        summary.playlists_created,
                    ));
                }
                Err(e) => toast.error(e),
            }
            staged_import.set(None);
            transfer_busy.set(false);
        });
    };

    rsx! {
        SettingsSubpage { title: "Server",
            div { class: "space-y-4 p-3 bg-base-200 rounded-lg",
                SettingsRow { label: "Server type",
                    if embedded_mode {
                        span { "Embedded (this device)" }
                    } else {
                        span { "Remote" }
                    }
                }
                SettingsRow { label: "Server URL",
                    if let Some(url) = config().server_url.clone() {
                        span { class: "font-mono", "{url}" }
                    } else {
                        span { class: "text-muted", "Not configured" }
                    }
                }
                if embedded_mode {
                    SettingsRow { label: "Status",
                        div { class: "flex items-center gap-2",
                            match live_state() {
                                EmbeddedState::Running { port } => rsx! {
                                    span { "Running (port {port})" }
                                },
                                EmbeddedState::Starting { .. } => rsx! {
                                    span { "Starting…" }
                                },
                                EmbeddedState::Stopped => rsx! {
                                    span { class: "text-muted", "Stopped" }
                                },
                                EmbeddedState::Failed { error } => rsx! {
                                    span { class: "text-error text-sm", "Failed: {error}" }
                                },
                                EmbeddedState::Unavailable => rsx! {
                                    span { class: "text-muted", "Unavailable in this build" }
                                },
                            }
                            // Heal a Stopped/Failed server in place.
                            if matches!(live_state(), EmbeddedState::Stopped | EmbeddedState::Failed { .. }) {
                                button {
                                    class: "btn btn-primary btn-xs",
                                    onclick: move |_| {
                                        if let Err(e) = embedded::ensure_started() {
                                            toast.error(format!("Couldn't start: {e}"));
                                        }
                                        live_state.set(embedded::state());
                                    },
                                    "Start"
                                }
                            }
                        }
                    }
                    if let Some(dir) = embedded::data_dir_display() {
                        SettingsRow { label: "Data location",
                            span { class: "font-mono text-xs break-all", "{dir}" }
                        }
                    }
                }
                // Admin-only: database transfer (both kinds — the full backup
                // story, and the embedded↔remote migration path), then the
                // reconciled runtime config, poll history, and logs.
                if is_admin() {
                    div { class: "space-y-2 pt-2",
                        SettingsRow {
                            label: "Export database",
                            hint: "A compressed snapshot of subscriptions, episodes, playlists and history. No passwords, no audio files — downloads re-fetch after an import.",
                            button {
                                class: "btn btn-primary btn-sm",
                                disabled: transfer_busy(),
                                onclick: export_db,
                                if transfer_busy() { "Working…" } else { "Export" }
                            }
                        }
                        SettingsRow {
                            label: "Import database",
                            hint: "Merges an export into this server: matching usernames merge, new users are created, nothing is replaced.",
                            if let Some((name, _)) = staged_import() {
                                div { class: "flex items-center gap-2 flex-wrap",
                                    span { class: "text-sm font-mono break-all", "{name}" }
                                    button {
                                        class: "btn btn-primary btn-sm",
                                        disabled: transfer_busy(),
                                        onclick: run_import,
                                        if transfer_busy() { "Importing…" } else { "Import now" }
                                    }
                                    button {
                                        class: "btn btn-ghost btn-sm",
                                        disabled: transfer_busy(),
                                        onclick: move |_| staged_import.set(None),
                                        "Cancel"
                                    }
                                }
                            } else {
                                label {
                                    class: "btn btn-primary btn-sm cursor-pointer",
                                    "Choose file"
                                    input {
                                        r#type: "file",
                                        accept: ".gz,.db,application/gzip,application/octet-stream",
                                        class: "hidden",
                                        onchange: move |evt| async move {
                                            let files = evt.files();
                                            let Some(file) = files.into_iter().next() else {
                                                return;
                                            };
                                            match file.read_bytes().await {
                                                Ok(bytes) => staged_import
                                                    .set(Some((file.name(), bytes.to_vec()))),
                                                Err(_) => toast.error("Could not read file"),
                                            }
                                        },
                                    }
                                }
                            }
                        }
                        SettingsRow { label: "Server configuration",
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| { nav.push(Route::ViewConfig {}); },
                                "View config"
                            }
                        }
                        SettingsRow { label: "Server Polling History",
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| { nav.push(Route::Polling {}); },
                                "View History"
                            }
                        }
                        // The embedded server writes no log file (its tracing
                        // events land in this app's device log instead), so the
                        // server-logs viewer would always be empty there.
                        if !embedded_mode {
                            SettingsRow { label: "Server Logs",
                                button {
                                    class: "btn btn-primary btn-sm",
                                    onclick: move |_| { nav.push(Route::ServerLogs {}); },
                                    "View Server Logs"
                                }
                            }
                        }
                        SettingsRow { label: "Server Errors",
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| { nav.push(Route::ServerErrors {}); },
                                "View Errors"
                            }
                        }
                    }
                }
            }
        }
    }
}
