use super::{DiscoverService, Items, SearchKey};
use crate::{bounded_text, gpodder, itunes};
use halogen_wire::{DiscoverProvider, DiscoverProviderError};
use std::collections::HashSet;

impl DiscoverService {
    pub(super) async fn fetch_snapshot(
        &self,
        key: &SearchKey,
    ) -> (Items, Vec<DiscoverProviderError>) {
        let results =
            futures_util::future::join_all(key.providers.iter().map(|provider| async move {
                (*provider, self.fetch_page_provider(key, *provider).await)
            }))
            .await;
        let mut podcasts = Vec::new();
        let mut episodes = Vec::new();
        let mut errors = Vec::new();
        for (provider, result) in results {
            match result {
                Ok(Items::Podcasts(mut items)) => podcasts.append(&mut items),
                Ok(Items::Episodes(mut items)) => episodes.append(&mut items),
                Err(message) => errors.push(DiscoverProviderError { provider, message }),
            }
        }
        let mut seen = HashSet::new();
        podcasts.retain(|item| {
            if item.feed_url.len() > 4096 {
                return false;
            }
            let Ok(mut url) = reqwest::Url::parse(&item.feed_url) else {
                return false;
            };
            if halogen_net::validate_url(&url).is_err() {
                return false;
            }
            url.set_fragment(None);
            seen.insert(url.to_string())
        });
        for item in &mut podcasts {
            item.title = bounded_text(&item.title, 512);
            item.description = bounded_text(&item.description, 1500);
            item.author = item.author.as_ref().map(|value| bounded_text(value, 512));
        }
        (
            if key.episodes {
                Items::Episodes(episodes)
            } else {
                Items::Podcasts(podcasts)
            },
            errors,
        )
    }

    async fn fetch_page_provider(
        &self,
        key: &SearchKey,
        provider: DiscoverProvider,
    ) -> Result<Items, String> {
        if key.episodes {
            match provider {
                DiscoverProvider::Itunes => self
                    .itunes_episodes(&key.query, 200)
                    .await
                    .map(Items::Episodes),
                DiscoverProvider::Gpodder => {
                    Err("gpodder.net does not support episode search".into())
                }
            }
        } else {
            match provider {
                DiscoverProvider::Itunes => {
                    itunes::search_limit(&self.client, &self.itunes_base, &key.query, 200)
                        .await
                        .map(Items::Podcasts)
                }
                DiscoverProvider::Gpodder => {
                    gpodder::search_at(&self.client, &self.gpodder_base, &key.query)
                        .await
                        .map(|items| Items::Podcasts(items.into_iter().take(20).collect()))
                }
            }
        }
    }
}
