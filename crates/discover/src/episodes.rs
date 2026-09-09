use halogen_wire::{
    DiscoverEpisodeItem, DiscoverEpisodeSearchData, DiscoverProvider, DiscoverProviderError,
};
use serde::Deserialize;
use std::collections::HashSet;

use crate::{DiscoverService, bounded_text, fetch_json, make_id};

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<Episode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Episode {
    kind: Option<String>,
    track_id: Option<u64>,
    track_name: Option<String>,
    collection_name: Option<String>,
    feed_url: Option<String>,
    episode_guid: Option<String>,
    description: Option<String>,
    release_date: Option<String>,
    track_time_millis: Option<u64>,
}

impl DiscoverService {
    /// Episode indexes are provider-specific; unsupported providers report an explicit partial error.
    pub async fn search_episodes(
        &self,
        q: &str,
        filter: Option<&[DiscoverProvider]>,
    ) -> DiscoverEpisodeSearchData {
        let selected = |provider| filter.is_none_or(|f| f.is_empty() || f.contains(&provider));
        let mut data = DiscoverEpisodeSearchData::default();
        if selected(DiscoverProvider::Gpodder) {
            data.errors.push(DiscoverProviderError {
                provider: DiscoverProvider::Gpodder,
                message: "gpodder.net does not support episode search".into(),
            });
        }
        if selected(DiscoverProvider::Itunes) {
            match self.itunes_episodes(q, 50).await {
                Ok(items) => data.items = items,
                Err(message) => data.errors.push(DiscoverProviderError {
                    provider: DiscoverProvider::Itunes,
                    message,
                }),
            }
        }
        data
    }

    pub(crate) async fn itunes_episodes(
        &self,
        q: &str,
        limit: usize,
    ) -> Result<Vec<DiscoverEpisodeItem>, String> {
        let limit_string = limit.to_string();
        let response: SearchResponse = fetch_json(
            &self.client,
            "iTunes",
            &self.itunes_base,
            &[
                ("media", "podcast"),
                ("entity", "podcastEpisode"),
                ("limit", limit_string.as_str()),
                ("term", q),
            ],
        )
        .await?;
        let mut seen = HashSet::new();
        Ok(response
            .results
            .into_iter()
            .filter_map(|item| {
                if item.kind.as_deref() != Some("podcast-episode") {
                    return None;
                }
                let feed_url = item.feed_url.filter(|s| !s.trim().is_empty())?;
                if feed_url.len() > 4096 {
                    return None;
                }
                let url = reqwest::Url::parse(&feed_url).ok()?;
                halogen_net::validate_url(&url).ok()?;
                let title = item.track_name.filter(|s| !s.trim().is_empty())?;
                let key = item
                    .episode_guid
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| item.track_id.map(|id| id.to_string()))?;
                let id = format!(
                    "episode-{}",
                    make_id(DiscoverProvider::Itunes, &format!("{feed_url}\0{key}"))
                );
                if !seen.insert(id.clone()) {
                    return None;
                }
                Some(DiscoverEpisodeItem {
                    id,
                    provider: DiscoverProvider::Itunes,
                    title: bounded_text(&title, 512),
                    feed_url,
                    podcast_title: bounded_text(
                        &item
                            .collection_name
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or_else(|| url.host_str().unwrap_or("Podcast").to_owned()),
                        512,
                    ),
                    description: bounded_text(&item.description.unwrap_or_default(), 65536),
                    guid: item.episode_guid.map(|value| bounded_text(&value, 4096)),
                    published_at: item
                        .release_date
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                        .map(|date| date.to_rfc3339()),
                    duration_seconds: item
                        .track_time_millis
                        .and_then(|ms| u32::try_from(ms / 1000).ok()),
                })
            })
            .take(limit)
            .collect())
    }
}
