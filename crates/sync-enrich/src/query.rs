//! The cache-side episode query model: the `EpisodeOrder`/`EpisodeQueryFilter`/
//! `EpisodeQuery` value types plus the shared filter/sort/paginate algorithm the
//! stores run over the cached pool. Kept out of `mod.rs` (the `LocalStore` trait)
//! since these are the request/predicate types, not the trait itself.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use halogen_wire::{DownloadStatus, EpisodeData, PlaybackStatus};

/// Field an episode page is ordered by. Mirrors the server's `order_by` values
/// the client actually uses; the UI `SortField` maps onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EpisodeOrder {
    /// Newest-first feed ordering (the `/latest` default).
    #[default]
    PublishedAt,
    /// Alphabetical by title.
    Title,
    /// Row creation time.
    CreatedAt,
    /// Last-updated time.
    UpdatedAt,
    /// Episode length (`duration_secs`); unknown lengths (`None`) sort as the
    /// smallest value, matching SQL `NULL` ordering on the server.
    Duration,
}

/// Predicate over the cached episode pool — the cache-side mirror of the server `FilterParams`. Empty fields =
/// no constraint. Multi-select facets (playback / download status) are OR within the facet, AND across facets.
/// `search` matches title, description, or the podcast name (resolved via [`Self::podcast_names`]),
/// case-insensitively.
#[derive(Debug, Clone, Default)]
pub struct EpisodeQueryFilter {
    pub search: Option<String>,
    pub podcast_id: Option<i32>,
    pub published_after: Option<DateTime<Utc>>,
    pub playback_status: Vec<PlaybackStatus>,
    pub download_status: Vec<DownloadStatus>,
    /// Restrict to a set of ids (e.g. the device-download set).
    pub ids: Option<HashSet<i32>>,
    /// Podcast id → title, used ONLY so `search` can match the show name. Episode rows no longer carry the
    /// nested podcast join, so the parent name is resolved from the EpisodeState podcast pool by the caller
    /// (see `read_cache`) and populated here only when a search is active. It's a resolution aid, not a
    /// constraint, so [`Self::is_empty`] ignores it (an empty map never narrows the result).
    pub podcast_names: HashMap<i32, String>,
}

impl EpisodeQueryFilter {
    /// True when nothing is constrained — lets both stores take their fast indexed
    /// `published_at` path (native SQL `LIMIT/OFFSET`, web `idx_pub` cursor) and
    /// O(1) count instead of loading the whole pool.
    pub fn is_empty(&self) -> bool {
        self.search.is_none()
            && self.podcast_id.is_none()
            && self.published_after.is_none()
            && self.playback_status.is_empty()
            && self.download_status.is_empty()
            && self.ids.is_none()
    }

    /// Whether an episode satisfies the predicate.
    pub fn matches(&self, ep: &EpisodeData) -> bool {
        if let Some(s) = &self.search {
            let q = s.to_lowercase();
            let in_title = ep.title.to_lowercase().contains(&q);
            let in_desc = ep
                .description
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q));
            let in_podcast = self
                .podcast_names
                .get(&ep.podcast_id)
                .is_some_and(|t| t.to_lowercase().contains(&q));
            if !(in_title || in_desc || in_podcast) {
                return false;
            }
        }
        if let Some(pid) = self.podcast_id
            && ep.podcast_id != pid
        {
            return false;
        }
        if let Some(after) = self.published_after {
            match ep.published_at {
                Some(p) if p >= after => {}
                _ => return false,
            }
        }
        if !self.playback_status.is_empty() && !self.playback_status.contains(&ep.playback_status) {
            return false;
        }
        if !self.download_status.is_empty() && !self.download_status.contains(&ep.download_status) {
            return false;
        }
        if let Some(ids) = &self.ids
            && !ids.contains(&ep.id)
        {
            return false;
        }
        true
    }
}

/// A request for one page of episodes from the local cache: filtered, ordered,
/// then sliced the same way the server pages them so the cache view tracks the
/// server's ordering as pages fill the pool.
#[derive(Debug, Clone, Default)]
pub struct EpisodeQuery {
    /// Column to order by.
    pub order_by: EpisodeOrder,
    /// Descending when true (newest/Z-A first).
    pub descending: bool,
    /// 0-based page index.
    pub page: i32,
    /// Page size.
    pub size: i32,
    /// Predicate applied before ordering + slicing.
    pub filter: EpisodeQueryFilter,
}

/// Filter, then sort the matching set the way `EpisodeQuery` asks, then slice the
/// page. Shared by the web store and the native non-fast path so cache ordering
/// matches the server (and the native indexed path). `id` is the deterministic
/// tiebreaker; `None` `published_at` sorts last under DESC.
pub fn filter_sort_paginate(eps: Vec<EpisodeData>, q: &EpisodeQuery) -> Vec<EpisodeData> {
    let mut eps: Vec<EpisodeData> = eps.into_iter().filter(|e| q.filter.matches(e)).collect();
    eps.sort_by(|a, b| {
        let cmp = match q.order_by {
            EpisodeOrder::PublishedAt => a.published_at.cmp(&b.published_at),
            EpisodeOrder::Title => a.title.cmp(&b.title),
            EpisodeOrder::CreatedAt => a.created_at.cmp(&b.created_at),
            EpisodeOrder::UpdatedAt => a.updated_at.cmp(&b.updated_at),
            EpisodeOrder::Duration => a.duration_secs.cmp(&b.duration_secs),
        }
        .then_with(|| a.id.cmp(&b.id));
        if q.descending { cmp.reverse() } else { cmp }
    });
    let size = q.size.max(1) as usize;
    let start = (q.page.max(0) as usize).saturating_mul(size);
    eps.into_iter().skip(start).take(size).collect()
}

/// Count the episodes matching a filter — drives `has_more` (is there more cached
/// pool to render before fetching the next server page?).
pub fn filter_count(eps: &[EpisodeData], filter: &EpisodeQueryFilter) -> usize {
    eps.iter().filter(|e| filter.matches(e)).count()
}

#[cfg(test)]
mod tests;
