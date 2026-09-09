//! Proxy online podcast-directory searches through `/api/v1/discover/*`; clients never contact providers or fetch their
//! artwork. Providers run concurrently and failures become per-provider DiscoverProviderError entries. New providers
//! need an enum variant, a run match arm, and a module.

mod episodes;
mod gpodder;
mod itunes;
mod pages;
mod preview;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use halogen_wire::{
    DiscoverProvider, DiscoverProviderError, DiscoverProviderInfo, DiscoverProvidersData,
    DiscoverResultItem, DiscoverSearchData,
};

/// Every provider this build knows about. Extend here when adding one.
const PROVIDERS: [DiscoverProvider; 2] = [DiscoverProvider::Itunes, DiscoverProvider::Gpodder];

/// Upper bound on a result's description so the detail page / route store stay
/// small (the list only shows ~3 clamped lines anyway).
const MAX_DESCRIPTION: usize = 1500;

/// Shared provider fetch: `GET base?<params>`, fail with a provider-prefixed
/// message on a non-2xx status, otherwise decode the JSON body into `T`. Each
/// provider supplies only its query params + response shape + row mapping.
pub(crate) async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    provider: &str,
    base: &str,
    params: &[(&str, &str)],
) -> Result<T, String> {
    let url = reqwest::Url::parse(base).map_err(|e| e.to_string())?;
    halogen_net::validate_url(&url)?;
    let response = client
        .get(base)
        .query(params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{provider} returned {}", response.status()));
    }
    let body = bounded_body(response, 2 * 1024 * 1024).await?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Compute the synthetic, stable id for a result: `provider-<hash(feed_url)>`. Deterministic within a running
/// server (fixed-seed `DefaultHasher`), which is all the detail page needs — results are ephemeral and the
/// client only echoes this string back to look the row up in its in-memory store.
fn make_id(provider: DiscoverProvider, feed_url: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    provider.as_str().hash(&mut hasher);
    feed_url.hash(&mut hasher);
    format!("{}-{:016x}", provider.as_str(), hasher.finish())
}

/// Truncate a description to [`MAX_DESCRIPTION`] chars, never splitting a char.
fn clamp_description(s: String) -> String {
    match s.char_indices().nth(MAX_DESCRIPTION) {
        Some((idx, _)) => {
            let mut out = s[..idx].to_string();
            out.push('…');
            out
        }
        None => s,
    }
}

/// Searches external podcast directories on behalf of the UI.
#[derive(Clone)]
pub struct DiscoverService {
    client: reqwest::Client,
    pages: std::sync::Arc<std::sync::Mutex<pages::PageCache>>,
    /// Upstream iTunes search endpoint (real host in prod; a wiremock URL in tests).
    itunes_base: String,
    /// Upstream gpodder.net search endpoint (real host in prod; wiremock in tests).
    gpodder_base: String,
}

impl DiscoverService {
    /// Build with explicit provider base URLs. Production passes the configured
    /// (real) endpoints; the integration harness points these at a wiremock
    /// server so the real `/discover` path runs without external calls.
    pub fn with_bases(itunes_base: String, gpodder_base: String) -> Self {
        let client = halogen_net::guarded_client_builder()
            // Bound a hung provider: connect fast, give the whole call ≤8s.
            .redirect(halogen_net::guarded_redirect_policy())
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(5))
            .gzip(true)
            // Some providers (e.g. PodcastIndex, future) require a User-Agent.
            .user_agent(halogen_net::user_agent("podcast-search"))
            // Routes through the SSRF DNS guard like every other outbound client.
            .build()
            .expect("build discover http client");
        Self {
            client,
            pages: Default::default(),
            itunes_base,
            gpodder_base,
        }
    }

    /// What the UI renders toggle chips for. All shipped providers are keyless
    /// and therefore always available.
    pub fn providers_info(&self) -> DiscoverProvidersData {
        DiscoverProvidersData {
            providers: PROVIDERS
                .iter()
                .map(|p| DiscoverProviderInfo {
                    id: *p,
                    label: p.label().to_string(),
                    available: true,
                    default_enabled: true,
                })
                .collect(),
        }
    }

    /// Run `q` against the selected providers concurrently and merge the results. `filter` of `None` or an
    /// empty slice means "all available providers". A provider that errors contributes a
    /// [`DiscoverProviderError`]; the rest still return — the search itself always succeeds.
    pub async fn search(&self, q: &str, filter: Option<&[DiscoverProvider]>) -> DiscoverSearchData {
        let all = filter.is_none_or(|f| f.is_empty());
        let selected: Vec<DiscoverProvider> = PROVIDERS
            .iter()
            .copied()
            .filter(|p| all || filter.is_some_and(|f| f.contains(p)))
            .collect();

        let futures = selected
            .into_iter()
            .map(|provider| async move { (provider, self.run(provider, q).await) });
        let results = futures_util::future::join_all(futures).await;

        let mut items = Vec::new();
        let mut errors = Vec::new();
        for (provider, result) in results {
            match result {
                Ok(mut v) => items.append(&mut v),
                Err(message) => errors.push(DiscoverProviderError { provider, message }),
            }
        }
        DiscoverSearchData { items, errors }
    }

    /// Dispatch to a single provider, then normalize: clamp descriptions and
    /// drop intra-provider duplicate feeds.
    async fn run(
        &self,
        provider: DiscoverProvider,
        q: &str,
    ) -> Result<Vec<DiscoverResultItem>, String> {
        let raw = match provider {
            DiscoverProvider::Itunes => {
                itunes::search_at(&self.client, &self.itunes_base, q).await?
            }
            DiscoverProvider::Gpodder => {
                gpodder::search_at(&self.client, &self.gpodder_base, q).await?
            }
        };
        let mut seen = HashSet::new();
        let items = raw
            .into_iter()
            .filter(|item| seen.insert(item.feed_url.clone()))
            .map(|mut item| {
                item.description = clamp_description(item.description);
                item
            })
            .collect();
        Ok(items)
    }
}

#[cfg(test)]
mod tests;

async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("Remote response exceeds size limit".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err("Remote response exceeds size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn bounded_text(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}
