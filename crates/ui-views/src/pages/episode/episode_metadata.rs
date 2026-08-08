//! Read-only episode metadata page (`/episodes/:id/metadata`).
//!
//! Renders EVERY field of the [`EpisodeData`] row, grouped into labelled
//! sections (same shape as `pages/view_config`). The nested embeds (`podcast` /
//! `playback` / `chapters`) are summarized as one row each, not expanded.
//! Reached from the episode kebab menu's "View metadata". The row comes from
//! the cached pool, with the usual deep-link fetch on a cold-store miss.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use halogen_wire::{EpisodeData, EpisodeInclude};

use crate::components::BackButton;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    deep_link_placeholder, use_config, use_connection, use_deep_link_resource, use_dispatch,
    use_episodes,
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
    v.clone().unwrap_or_else(|| "None".to_string())
}

fn dt(v: &DateTime<Utc>) -> String {
    v.to_rfc3339()
}

fn opt_dt(v: &Option<DateTime<Utc>>) -> String {
    v.as_ref().map(dt).unwrap_or_else(|| "None".to_string())
}

/// Partition the flat [`EpisodeData`] into display sections. Every field appears
/// exactly once (embeds summarized, not expanded) — guarded by the unit test.
fn sections(e: &EpisodeData) -> Vec<Section> {
    vec![
        Section {
            title: "Episode",
            rows: vec![
                Row {
                    label: "ID",
                    value: e.id.to_string(),
                },
                Row {
                    label: "Podcast ID",
                    value: e.podcast_id.to_string(),
                },
                Row {
                    label: "Title",
                    value: e.title.clone(),
                },
                Row {
                    label: "Description",
                    value: opt(&e.description),
                },
                Row {
                    label: "GUID",
                    value: opt(&e.guid),
                },
            ],
        },
        Section {
            title: "Media",
            rows: vec![
                Row {
                    label: "Content URL",
                    value: e.content_url.clone(),
                },
                Row {
                    label: "Duration (s)",
                    value: e
                        .duration_secs
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Unknown".to_string()),
                },
            ],
        },
        Section {
            title: "Download",
            rows: vec![
                Row {
                    label: "Status",
                    value: format!("{:?}", e.download_status),
                },
                Row {
                    label: "Started at",
                    value: opt_dt(&e.download_started_at),
                },
                Row {
                    label: "Downloaded at",
                    value: opt_dt(&e.downloaded_at),
                },
                Row {
                    label: "Attempts",
                    value: e.download_attempts.to_string(),
                },
                Row {
                    label: "Size (bytes)",
                    value: e
                        .download_size
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "None".to_string()),
                },
                Row {
                    label: "Content file path",
                    value: opt(&e.content_file_path),
                },
            ],
        },
        Section {
            title: "Artwork",
            rows: vec![
                Row {
                    label: "Art URL",
                    value: opt(&e.art_url),
                },
                Row {
                    label: "Art file path",
                    value: opt(&e.art_file_path),
                },
            ],
        },
        Section {
            // Embeds — one summary row each (count / cursor / title), never the
            // full nested structs.
            title: "Playback & embeds",
            rows: vec![
                Row {
                    label: "Playback status",
                    value: format!("{:?}", e.playback_status),
                },
                Row {
                    label: "Playback cursor",
                    value: e
                        .playback
                        .as_ref()
                        .map(|p| {
                            format!(
                                "{}s{}",
                                p.cursor,
                                if p.completed { " (completed)" } else { "" }
                            )
                        })
                        .unwrap_or_else(|| "None".to_string()),
                },
                Row {
                    label: "Podcast",
                    value: e
                        .podcast
                        .as_ref()
                        .map(|p| p.title.clone())
                        .unwrap_or_else(|| "Not embedded".to_string()),
                },
                Row {
                    label: "Chapters",
                    value: e
                        .chapters
                        .as_ref()
                        .map(|c| format!("{} chapter(s)", c.len()))
                        .unwrap_or_else(|| "Not loaded".to_string()),
                },
            ],
        },
        Section {
            title: "Record",
            rows: vec![
                Row {
                    label: "Published at",
                    value: opt_dt(&e.published_at),
                },
                Row {
                    label: "Created at",
                    value: dt(&e.created_at),
                },
                Row {
                    label: "Updated at",
                    value: dt(&e.updated_at),
                },
            ],
        },
    ]
}

#[component]
pub fn EpisodeMetadata(id: i32) -> Element {
    let app_state = use_episodes();
    let dispatch = use_dispatch();
    let config = use_config();

    // Episodes aren't bulk-hydrated; fetch + cache this one on a deep-link miss
    // (mirrors `EpisodeDetail`, including the Playback include so re-caching
    // doesn't blank an embedded cursor).
    let load_failed = use_deep_link_resource(
        config,
        move || app_state.read().episode(id).is_some(),
        move |client| async move {
            let ep = client
                .get_episode(id, &[EpisodeInclude::Playback])
                .await
                .map_err(|e| e.to_string())?;
            commands::cache_episodes(&dispatch, vec![ep]);
            Ok(())
        },
    );

    let episode = app_state.read().episode(id).cloned();
    let is_offline = use_connection().read().is_offline();

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row — stays put while the metadata scrolls.
            div { class: "p-2", BackButton {} }
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-8",
                h1 { class: "text-3xl font-bold mb-2", "Episode Metadata" }
                if let Some(e) = episode {
                    p { class: "text-muted mb-4", "{e.title}" }
                    for section in sections(&e) {
                        div { class: "space-y-2 p-4 bg-base-200 rounded-lg",
                            h2 { class: "text-xl font-semibold mb-2", "{section.title}" }
                            for row in section.rows {
                                div {
                                    class: "flex items-start justify-between gap-4 py-1 border-b border-base-300 last:border-0",
                                    span { class: "text-muted", "{row.label}" }
                                    span { class: "font-mono text-right break-all", "{row.value}" }
                                }
                            }
                        }
                    }
                } else {
                    p { class: "text-muted",
                        {deep_link_placeholder(load_failed(), is_offline, "episode")}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halogen_wire::{DownloadStatus, PlaybackStatus};

    fn sample() -> EpisodeData {
        EpisodeData {
            id: 1,
            podcast_id: 2,
            title: "T".to_string(),
            description: None,
            content_url: "https://example.com/e.mp3".to_string(),
            guid: None,
            art_url: None,
            published_at: None,
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    /// Every `EpisodeData` field is surfaced exactly once across all sections
    /// (22 fields → 22 rows; the podcast/playback/chapters embeds are summarized
    /// as one row each).
    #[test]
    fn sections_cover_all_fields() {
        let groups = sections(&sample());
        let total_rows: usize = groups.iter().map(|s| s.rows.len()).sum();
        assert_eq!(
            total_rows, 22,
            "every EpisodeData field should map to one row"
        );
    }
}
