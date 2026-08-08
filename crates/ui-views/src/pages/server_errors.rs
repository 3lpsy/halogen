//! Server Errors page (`/admin/errors`, admin-only).
//!
//! Renders the server's persisted failure histories from
//! `GET /admin/server-errors`: podcast RSS sync failures and episode
//! media-download failures, newest first, each with the failure reason the
//! server recorded. Online-only (the histories live in the server DB).

use dioxus::prelude::*;
use halogen_wire::{EpisodeDownloadErrorData, PodcastSyncErrorData};

use crate::Route;
use crate::components::BackButton;
use halogen_ui_icons::ClockRotateLeft;
use halogen_ui_state::hooks::use_config;

#[component]
pub fn ServerErrors() -> Element {
    let config = use_config();
    // Bumping this refetches both histories.
    let mut refresh = use_signal(|| 0u32);

    let errors = use_resource(move || {
        let _ = refresh();
        // None when unconfigured OR manually offline — "Go Offline" suppresses
        // the fetch like every other online-only surface.
        let client = config.read().api_client_or_err();
        async move {
            let client = client?;
            client.get_server_errors().await.map_err(|e| e.to_string())
        }
    });

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            div { class: "p-2 flex items-center justify-between gap-2",
                BackButton {}
                button {
                    class: "btn btn-ghost btn-sm btn-square",
                    "aria-label": "Refresh",
                    title: "Refresh",
                    onclick: move |_| refresh += 1,
                    ClockRotateLeft { class: "w-4 h-4" }
                }
            }
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-6",
                h1 { class: "text-3xl font-bold", "Server Errors" }
                p { class: "text-muted text-sm",
                    "Recent failures recorded by the server: feed syncs that couldn't be fetched or parsed, and episode downloads that failed. Newest first."
                }

                match &*errors.read_unchecked() {
                    None => rsx! {
                        div { class: "flex justify-center py-16",
                            span { class: "loading loading-spinner" }
                        }
                    },
                    Some(Err(e)) => rsx! {
                        p { class: "text-error text-sm", "Couldn't load server errors: {e}" }
                    },
                    Some(Ok(data)) => rsx! {
                        RssSyncErrorsSection { rows: data.rss_sync.clone() }
                        DownloadErrorsSection { rows: data.episode_downloads.clone() }
                    },
                }
            }
        }
    }
}

/// Podcast RSS sync failures. Each row links to its podcast.
#[component]
fn RssSyncErrorsSection(rows: Vec<PodcastSyncErrorData>) -> Element {
    rsx! {
        div { class: "space-y-2",
            h2 { class: "text-xl font-semibold", "Feed sync failures" }
            if rows.is_empty() {
                p { class: "text-muted text-sm", "None recorded." }
            }
            for row in rows {
                div {
                    key: "rss-{row.id}",
                    class: "p-3 bg-base-200 rounded-lg space-y-1",
                    div { class: "flex items-start justify-between gap-2",
                        Link {
                            to: Route::PodcastDetail { id: row.podcast_id },
                            class: "font-medium text-primary hover:underline truncate",
                            {row.podcast_title.clone().unwrap_or_else(|| format!("Podcast #{}", row.podcast_id))}
                        }
                        span { class: "text-xs text-muted whitespace-nowrap",
                            {row.created_at.format("%b %d, %H:%M").to_string()}
                        }
                    }
                    p { class: "text-sm text-base-content/80 font-mono break-all", "{row.reason}" }
                }
            }
        }
    }
}

/// Episode media-download failures. Each row links to its episode.
#[component]
fn DownloadErrorsSection(rows: Vec<EpisodeDownloadErrorData>) -> Element {
    rsx! {
        div { class: "space-y-2",
            h2 { class: "text-xl font-semibold", "Episode download failures" }
            if rows.is_empty() {
                p { class: "text-muted text-sm", "None recorded." }
            }
            for row in rows {
                div {
                    key: "dl-{row.id}",
                    class: "p-3 bg-base-200 rounded-lg space-y-1",
                    div { class: "flex items-start justify-between gap-2",
                        Link {
                            to: Route::EpisodeDetail { id: row.episode_id },
                            class: "font-medium text-primary hover:underline truncate",
                            {row.episode_title.clone().unwrap_or_else(|| format!("Episode #{}", row.episode_id))}
                        }
                        span { class: "text-xs text-muted whitespace-nowrap",
                            {row.created_at.format("%b %d, %H:%M").to_string()}
                        }
                    }
                    p { class: "text-sm text-base-content/80 font-mono break-all", "{row.reason}" }
                }
            }
        }
    }
}
