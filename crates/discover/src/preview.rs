use halogen_wire::{
    DiscoverEpisodeItem, DiscoverPodcastData, DiscoverProvider, DiscoverResultItem,
};
use std::collections::HashSet;

use crate::{DiscoverService, bounded_text, make_id};

mod body;

impl DiscoverService {
    /// Fetch a bounded feed without creating subscriptions or library records.
    pub async fn podcast_preview(
        &self,
        feed_url: &str,
        provider: DiscoverProvider,
    ) -> Result<DiscoverPodcastData, String> {
        if feed_url.len() > 4096 {
            return Err("Feed URL exceeds 4096 bytes".into());
        }
        let url = reqwest::Url::parse(feed_url).map_err(|_| "Invalid feed URL")?;
        halogen_net::validate_url(&url)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Feed returned {}", response.status()));
        }
        let body = body::read_response(response).await?;
        parse_feed(&body, feed_url, provider)
    }
}

fn parse_feed(
    body: &[u8],
    feed_url: &str,
    provider: DiscoverProvider,
) -> Result<DiscoverPodcastData, String> {
    let channel = rss::Channel::read_from(body).map_err(|_| "Invalid RSS feed")?;
    let podcast = DiscoverResultItem {
        id: make_id(provider, feed_url),
        provider,
        title: bounded_text(
            if channel.title().trim().is_empty() {
                feed_url
            } else {
                channel.title()
            },
            512,
        ),
        feed_url: feed_url.to_owned(),
        description: bounded_text(
            std::iter::once(channel.description())
                .chain(channel.itunes_ext().and_then(|ext| ext.summary()))
                .map(str::trim)
                .find(|value| !value.is_empty())
                .unwrap_or_default(),
            65536,
        ),
        author: channel
            .itunes_ext()
            .and_then(|ext| ext.author())
            .map(|value| bounded_text(value, 512)),
    };
    let mut seen = HashSet::new();
    let episodes = channel
        .items()
        .iter()
        .filter_map(|item| {
            let title = item.title().filter(|s| !s.trim().is_empty())?;
            let guid = item.guid().map(|guid| guid.value().to_owned());
            let key = guid
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| item.enclosure().map(|e| e.url()))
                .or_else(|| item.link())?;
            let id = format!(
                "episode-{}",
                make_id(provider, &format!("{feed_url}\0{key}"))
            );
            if !seen.insert(id.clone()) {
                return None;
            }
            Some(DiscoverEpisodeItem {
                id,
                provider,
                title: bounded_text(title, 512),
                feed_url: feed_url.to_owned(),
                podcast_title: podcast.title.clone(),
                description: bounded_text(
                    item.content()
                        .or_else(|| item.description())
                        .unwrap_or_default(),
                    65536,
                ),
                guid: guid.map(|value| bounded_text(&value, 4096)),
                published_at: item
                    .pub_date()
                    .and_then(|date| chrono::DateTime::parse_from_rfc2822(date).ok())
                    .map(|date| date.to_rfc3339()),
                duration_seconds: item
                    .itunes_ext()
                    .and_then(|ext| ext.duration())
                    .and_then(duration_seconds),
            })
        })
        .take(body::MAX_EPISODES)
        .collect();
    Ok(DiscoverPodcastData { podcast, episodes })
}

fn duration_seconds(value: &str) -> Option<u32> {
    let parts: Vec<_> = value.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    parts.iter().try_fold(0u32, |total, part| {
        total.checked_mul(60)?.checked_add(part.parse().ok()?)
    })
}

#[cfg(test)]
mod tests;
