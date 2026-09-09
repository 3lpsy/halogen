use std::collections::VecDeque;
use std::time::{Duration, Instant};

use halogen_wire::*;
use rand::Rng;

use crate::{DiscoverService, PROVIDERS};

mod fetch;

const PAGE_SIZE: usize = 25;
const MAX_SNAPSHOTS: usize = 32;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
const SNAPSHOT_TTL: Duration = Duration::from_secs(600);
const CURSOR_ERROR: &str = "Search cursor expired or invalid; restart the search";

#[derive(Clone, PartialEq)]
struct SearchKey {
    query: String,
    providers: Vec<DiscoverProvider>,
    episodes: bool,
}

#[derive(Clone, serde::Serialize)]
enum Items {
    Podcasts(Vec<DiscoverResultItem>),
    Episodes(Vec<DiscoverEpisodeItem>),
}

struct Snapshot {
    id: String,
    key: SearchKey,
    created: Instant,
    items: Items,
    errors: Vec<DiscoverProviderError>,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct PageCache {
    snapshots: VecDeque<Snapshot>,
}

impl DiscoverService {
    pub async fn podcast_page(
        &self,
        params: DiscoverPageParams,
    ) -> Result<DiscoverPodcastPageData, String> {
        let (items, errors, page) = self.page(params, false).await?;
        let Items::Podcasts(items) = items else {
            unreachable!()
        };
        Ok(DiscoverPodcastPageData {
            items,
            errors,
            page,
        })
    }

    pub async fn episode_page(
        &self,
        params: DiscoverPageParams,
    ) -> Result<DiscoverEpisodePageData, String> {
        let (items, errors, page) = self.page(params, true).await?;
        let Items::Episodes(items) = items else {
            unreachable!()
        };
        Ok(DiscoverEpisodePageData {
            items,
            errors,
            page,
        })
    }

    async fn page(
        &self,
        params: DiscoverPageParams,
        episodes: bool,
    ) -> Result<(Items, Vec<DiscoverProviderError>, DiscoverPageInfo), String> {
        if params.q.len() > 1024 {
            return Err("Search query exceeds size limit".into());
        }
        let query = params.q.split_whitespace().collect::<Vec<_>>().join(" ");
        if query.is_empty() || query.chars().count() > 256 {
            return Err("Search query must contain 1–256 characters".into());
        }
        let key = SearchKey {
            query,
            episodes,
            providers: PROVIDERS
                .into_iter()
                .filter(|provider| {
                    params.providers.as_ref().is_none_or(|providers| {
                        providers.is_empty() || providers.contains(provider)
                    })
                })
                .collect(),
        };
        if let Some(cursor) = params.cursor {
            return self
                .pages
                .lock()
                .map_err(|_| "Search cache unavailable")?
                .read(&key, &cursor);
        }
        let (items, errors) = self.fetch_snapshot(&key).await;
        let bytes = serde_json::to_vec(&items).map_err(|e| e.to_string())?.len();
        if bytes > MAX_CACHE_BYTES {
            return Err("Search response exceeds cache size limit".into());
        }
        let snapshot = Snapshot {
            id: format!("{:032x}", rand::thread_rng().r#gen::<u128>()),
            key,
            created: Instant::now(),
            items,
            errors,
            bytes,
        };
        let result = snapshot.page(0)?;
        let mut cache = self.pages.lock().map_err(|_| "Search cache unavailable")?;
        cache.prune();
        while cache.snapshots.len() >= MAX_SNAPSHOTS
            || cache.snapshots.iter().map(|s| s.bytes).sum::<usize>() + bytes > MAX_CACHE_BYTES
        {
            cache.snapshots.pop_front();
        }
        cache.snapshots.push_back(snapshot);
        Ok(result)
    }
}

impl PageCache {
    fn prune(&mut self) {
        self.snapshots
            .retain(|snapshot| snapshot.created.elapsed() < SNAPSHOT_TTL);
    }

    fn read(
        &mut self,
        key: &SearchKey,
        cursor: &str,
    ) -> Result<(Items, Vec<DiscoverProviderError>, DiscoverPageInfo), String> {
        self.prune();
        if cursor.len() > 40 {
            return Err(CURSOR_ERROR.into());
        }
        let (id, offset) = cursor.split_once(':').ok_or(CURSOR_ERROR)?;
        let offset: usize = offset.parse().map_err(|_| CURSOR_ERROR)?;
        if id.len() != 32
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
            || offset == 0
            || !offset.is_multiple_of(PAGE_SIZE)
        {
            return Err(CURSOR_ERROR.into());
        }
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.id == id && snapshot.key == *key)
            .ok_or(CURSOR_ERROR)?
            .page(offset)
    }
}

impl Snapshot {
    fn page(
        &self,
        offset: usize,
    ) -> Result<(Items, Vec<DiscoverProviderError>, DiscoverPageInfo), String> {
        let length = match &self.items {
            Items::Podcasts(items) => items.len(),
            Items::Episodes(items) => items.len(),
        };
        if offset > 0 && offset >= length {
            return Err(CURSOR_ERROR.into());
        }
        let end = (offset + PAGE_SIZE).min(length);
        let items = match &self.items {
            Items::Podcasts(items) => Items::Podcasts(items[offset..end].to_vec()),
            Items::Episodes(items) => Items::Episodes(items[offset..end].to_vec()),
        };
        let page = DiscoverPageInfo {
            has_more: end < length,
            next_cursor: (end < length).then(|| format!("{}:{end}", self.id)),
            result_limit: self
                .key
                .providers
                .iter()
                .map(|p| match p {
                    DiscoverProvider::Itunes => 200,
                    DiscoverProvider::Gpodder if !self.key.episodes => 20,
                    _ => 0,
                })
                .sum(),
        };
        Ok((items, self.errors.clone(), page))
    }
}

#[cfg(test)]
mod tests;
