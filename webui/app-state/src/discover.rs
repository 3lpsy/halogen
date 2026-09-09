//! Discovery remains ephemeral and separate from the synchronized library.

use halogen_wire::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum DiscoverMode {
    #[default]
    Podcasts,
    Episodes,
}

#[derive(Clone, Default, PartialEq)]
pub struct DiscoverState {
    pub query: String,
    pub mode: DiscoverMode,
    pub providers: Vec<DiscoverProvider>,
    pub results: Vec<DiscoverResultItem>,
    pub episodes: Vec<DiscoverEpisodeItem>,
    pub previews: Vec<DiscoverPodcastData>,
    pub page: Option<DiscoverPageInfo>,
    pub errors: Vec<DiscoverProviderError>,
    pub failure: Option<String>,
    pub loading: bool,
    pub searched: bool,
    pub generation: u64,
    pub scroll_top: f64,
}

impl DiscoverState {
    pub fn get(&self, id: &str) -> Option<&DiscoverResultItem> {
        self.results.iter().find(|r| r.id == id).or_else(|| {
            self.previews
                .iter()
                .map(|p| &p.podcast)
                .find(|p| p.id == id)
        })
    }

    pub fn episode(&self, id: &str) -> Option<&DiscoverEpisodeItem> {
        self.episodes
            .iter()
            .chain(self.previews.iter().flat_map(|p| p.episodes.iter()))
            .find(|e| e.id == id)
    }

    /// Invalidate in-flight requests whenever the search inputs change.
    pub fn reset(&mut self, query: String, mode: DiscoverMode, providers: Vec<DiscoverProvider>) {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            query,
            mode,
            providers,
            generation,
            ..Self::default()
        };
    }

    pub fn append_podcasts(&mut self, data: DiscoverPodcastPageData) {
        for item in data.items {
            if !self.results.iter().any(|old| old.feed_url == item.feed_url) {
                self.results.push(item);
            }
        }
        self.page = Some(data.page);
        self.errors = data.errors;
    }

    pub fn append_episodes(&mut self, data: DiscoverEpisodePageData) {
        for item in data.items {
            if !self.episodes.iter().any(|old| old.id == item.id) {
                self.episodes.push(item);
            }
        }
        self.page = Some(data.page);
        self.errors = data.errors;
    }
}

#[cfg(test)]
mod tests;
