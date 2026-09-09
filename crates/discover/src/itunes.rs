//! iTunes Search API provider. `GET
//! https://itunes.apple.com/search?media=podcast&entity=podcast&limit=25&term=<q>` — keyless, no auth. Returns
//! `collectionName` / `artistName` / `feedUrl`. It does **not** return a usable description, so results from
//! here have none.

use halogen_wire::{DiscoverProvider, DiscoverResultItem};
use serde::Deserialize;

use super::{fetch_json, make_id};

#[derive(Deserialize)]
struct ItunesResponse {
    #[serde(default)]
    results: Vec<ItunesResult>,
}

#[derive(Deserialize)]
struct ItunesResult {
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "feedUrl")]
    feed_url: Option<String>,
}

/// `base` is the iTunes Search endpoint (real host in prod; a wiremock URL in
/// tests). Query params are appended by `reqwest`.
pub async fn search_at(
    client: &reqwest::Client,
    base: &str,
    q: &str,
) -> Result<Vec<DiscoverResultItem>, String> {
    search_limit(client, base, q, 25).await
}

pub async fn search_limit(
    client: &reqwest::Client,
    base: &str,
    q: &str,
    limit: usize,
) -> Result<Vec<DiscoverResultItem>, String> {
    let limit_string = limit.to_string();
    let body: ItunesResponse = fetch_json(
        client,
        "iTunes",
        base,
        &[
            ("media", "podcast"),
            ("entity", "podcast"),
            ("limit", limit_string.as_str()),
            ("term", q),
        ],
    )
    .await?;

    let items = body
        .results
        .into_iter()
        .filter_map(|r| {
            // No feed URL ⇒ can't subscribe and can't key it — drop the row.
            let feed_url = r.feed_url.filter(|u| !u.trim().is_empty())?;
            let title = r.collection_name.filter(|t| !t.trim().is_empty())?;
            Some(DiscoverResultItem {
                id: make_id(DiscoverProvider::Itunes, &feed_url),
                provider: DiscoverProvider::Itunes,
                title,
                feed_url,
                description: String::new(),
                author: r.artist_name.filter(|a| !a.trim().is_empty()),
            })
        })
        .take(limit)
        .collect();

    Ok(items)
}
