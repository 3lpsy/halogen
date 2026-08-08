//! Read-only podcast metadata page (`/podcasts/:id/metadata`).
//!
//! Renders EVERY field of the [`PodcastData`] row, grouped into labelled
//! sections (same shape as `pages/view_config`). Reached from the podcast
//! kebab menu's "View metadata". The row comes from the cached pool, with the
//! usual deep-link fetch on a cold-store miss.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use halogen_wire::PodcastData;

use crate::components::BackButton;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    deep_link_placeholder, use_config, use_connection, use_deep_link_resource, use_dispatch,
    use_podcasts,
};

/// A row's rendered value: a single line, or one line per redirect hop.
enum Value {
    Text(String),
    /// Stacked lines (one per feed-redirect hop) — never joined into one blob.
    Lines(Vec<String>),
}

/// One labelled row in a section.
struct Row {
    label: &'static str,
    value: Value,
}

impl Row {
    fn text(label: &'static str, value: impl Into<String>) -> Self {
        Row {
            label,
            value: Value::Text(value.into()),
        }
    }
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

/// Split the stored redirect chain (a comma-joined CSV of hops, starting with
/// `feed_url` — see the wire docs) into numbered lines, one per hop. `None` /
/// empty renders as a single "None" line.
fn redirect_hops(v: &Option<String>) -> Value {
    let hops: Vec<String> = v
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(i, hop)| format!("{}. {hop}", i + 1))
        .collect();
    if hops.is_empty() {
        Value::Text("None".to_string())
    } else {
        Value::Lines(hops)
    }
}

/// Partition the flat [`PodcastData`] into display sections. Every field appears
/// exactly once (embeds summarized, not expanded) — guarded by the unit test.
fn sections(p: &PodcastData) -> Vec<Section> {
    vec![
        Section {
            title: "Podcast",
            rows: vec![
                Row::text("ID", p.id.to_string()),
                Row::text("Title", p.title.clone()),
                Row::text("Description", p.description.clone()),
                Row::text("Author", opt(&p.author)),
            ],
        },
        Section {
            title: "Feed",
            rows: vec![
                Row::text("Feed URL", p.feed_url.clone()),
                Row {
                    label: "Feed URL redirects",
                    value: redirect_hops(&p.feed_url_redirects),
                },
                Row::text("ETag", opt(&p.etag)),
                Row::text("Last modified", opt(&p.last_modified)),
                Row::text("Polled at", opt_dt(&p.polled_at)),
            ],
        },
        Section {
            title: "Artwork",
            rows: vec![
                Row::text("Art URL", opt(&p.art_url)),
                Row::text("Art file path", opt(&p.art_file_path)),
            ],
        },
        Section {
            title: "Polling config",
            rows: vec![
                Row::text(
                    "Config ID",
                    p.podcast_config_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "None".to_string()),
                ),
                // Embed — summarized, not expanded (it has its own edit page).
                Row::text(
                    "Config",
                    p.podcast_config
                        .as_ref()
                        .map(|c| format!("Embedded (config #{})", c.id))
                        .unwrap_or_else(|| "Not embedded".to_string()),
                ),
            ],
        },
        Section {
            title: "Record",
            rows: vec![
                Row::text(
                    "Episode count",
                    p.episode_count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Not computed".to_string()),
                ),
                Row::text("Created at", dt(&p.created_at)),
                Row::text("Updated at", dt(&p.updated_at)),
            ],
        },
    ]
}

#[component]
pub fn PodcastMetadata(id: i32) -> Element {
    let podcasts = use_podcasts();
    let dispatch = use_dispatch();
    let config = use_config();

    // Podcasts aren't bulk-held; fetch + cache this one on a deep-link miss
    // (mirrors `PodcastDetail`).
    let load_failed = use_deep_link_resource(
        config,
        move || podcasts.read().podcast(id).is_some(),
        move |client| async move {
            let p = client.get_podcast(id).await.map_err(|e| e.to_string())?;
            commands::cache_podcasts(&dispatch, vec![p]);
            Ok(())
        },
    );

    let podcast = podcasts.read().podcast(id).cloned();
    let is_offline = use_connection().read().is_offline();

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row — stays put while the metadata scrolls.
            div { class: "p-2", BackButton {} }
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-8",
                h1 { class: "text-3xl font-bold mb-2", "Podcast Metadata" }
                if let Some(p) = podcast {
                    p { class: "text-muted mb-4", "{p.title}" }
                    for section in sections(&p) {
                        div { class: "space-y-2 p-4 bg-base-200 rounded-lg",
                            h2 { class: "text-xl font-semibold mb-2", "{section.title}" }
                            for row in section.rows {
                                div {
                                    class: "flex items-start justify-between gap-4 py-1 border-b border-base-300 last:border-0",
                                    span { class: "text-muted", "{row.label}" }
                                    match row.value {
                                        Value::Text(value) => rsx! {
                                            span { class: "font-mono text-right break-all", "{value}" }
                                        },
                                        // One line per redirect hop, stacked vertically.
                                        Value::Lines(lines) => rsx! {
                                            div { class: "flex flex-col items-end gap-1",
                                                for line in lines {
                                                    span { class: "font-mono text-right break-all", "{line}" }
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                } else {
                    p { class: "text-muted",
                        {deep_link_placeholder(load_failed(), is_offline, "podcast")}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PodcastData {
        PodcastData {
            id: 1,
            title: "T".to_string(),
            description: "D".to_string(),
            feed_url: "https://example.com/feed".to_string(),
            art_url: None,
            author: None,
            polled_at: None,
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    /// Every `PodcastData` field is surfaced exactly once across all sections
    /// (16 fields → 16 rows; the config embed is summarized as one row).
    #[test]
    fn sections_cover_all_fields() {
        let groups = sections(&sample());
        let total_rows: usize = groups.iter().map(|s| s.rows.len()).sum();
        assert_eq!(
            total_rows, 16,
            "every PodcastData field should map to one row"
        );
    }

    /// The redirect chain renders one numbered line per hop, never one blob;
    /// `None` and empty both render "None".
    #[test]
    fn redirect_hops_split_into_numbered_lines() {
        match redirect_hops(&Some("https://a/feed, https://b/feed".to_string())) {
            Value::Lines(lines) => {
                assert_eq!(lines, vec!["1. https://a/feed", "2. https://b/feed"]);
            }
            Value::Text(_) => panic!("expected one line per hop"),
        }
        for empty in [None, Some(String::new()), Some(" , ".to_string())] {
            match redirect_hops(&empty) {
                Value::Text(t) => assert_eq!(t, "None"),
                Value::Lines(_) => panic!("expected None for {empty:?}"),
            }
        }
    }
}
