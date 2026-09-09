//! gpodder.net provider. `GET https://gpodder.net/search.json?q=<q>` — keyless, no auth. Returns a JSON array
//! of podcasts with `url` (the feed), `title`, `description`, `author`. We deliberately ignore the logo fields
//! (no artwork egress).

use halogen_wire::{DiscoverProvider, DiscoverResultItem};
use serde::Deserialize;

use super::{fetch_json, make_id};

#[derive(Deserialize)]
struct GpodderResult {
    url: Option<String>,
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<String>,
}

/// `base` is the gpodder.net search endpoint (real host in prod; a wiremock URL
/// in tests). The `q` param is appended by `reqwest`.
pub async fn search_at(
    client: &reqwest::Client,
    base: &str,
    q: &str,
) -> Result<Vec<DiscoverResultItem>, String> {
    // gpodder returns a bare JSON array, not an envelope.
    let body: Vec<GpodderResult> = fetch_json(client, "gpodder.net", base, &[("q", q)]).await?;

    let items = body
        .into_iter()
        .filter_map(|r| {
            let feed_url = r.url.filter(|u| !u.trim().is_empty())?;
            let title = r.title.filter(|t| !t.trim().is_empty())?;
            Some(DiscoverResultItem {
                id: make_id(DiscoverProvider::Gpodder, &feed_url),
                provider: DiscoverProvider::Gpodder,
                title,
                feed_url,
                description: r.description.unwrap_or_default(),
                author: r.author.filter(|a| !a.trim().is_empty()),
            })
        })
        // gpodder has no server-side limit param; cap client-side to match iTunes.
        .take(25)
        .collect();

    Ok(items)
}
