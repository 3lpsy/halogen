//! Worker-owned podcast and auto-playlist state has a separate signal so those writes notify podcast consumers without
//! rerendering unrelated episode data.

use std::collections::HashMap;

use halogen_wire::PodcastData;

/// Cached podcasts + their auto-playlist config. Its own slice, separate from `EpisodeState`; the worker is the sole
/// writer. `PartialEq` is load-bearing (same reason as on `EpisodeState`): this is read through a signal and sliced by
/// memos, so a derive that silently degraded to "always changed" would defeat the segmentation. The
/// `podcaststate_is_partialeq` canary guards it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PodcastState {
    /// Cached podcasts keyed by id — the pool filled lazily by the paged podcasts
    /// list (and on-demand detail fetches), mirroring `EpisodeState::episodes_by_id`.
    /// Not the whole subscription set; only what's been browsed/fetched.
    pub podcasts_by_id: HashMap<i32, PodcastData>,
    /// Playlist ids a podcast auto-adds new episodes to, keyed by podcast_id.
    /// Populated on demand by the "Configure auto-playlists" page (fetch when
    /// online, optimistic write when offline). In-memory only — not persisted to
    /// the local store; the durable record is the server + the outbox op.
    pub auto_playlists_by_podcast: HashMap<i32, Vec<i32>>,
    /// The podcast's auto-add insert-position override, keyed by podcast_id and
    /// cached alongside `auto_playlists_by_podcast`: `Some(true)` = start of
    /// the playlists, `Some(false)` = end, `None` = server default. Missing key
    /// = not yet fetched. `#[serde(default)]` keeps pre-field snapshots readable.
    #[serde(default)]
    pub auto_playlist_add_to_start_by_podcast: HashMap<i32, Option<bool>>,
}

impl PodcastState {
    /// Look up a cached podcast by id (O(1)) from the pool.
    pub fn podcast(&self, podcast_id: i32) -> Option<&PodcastData> {
        self.podcasts_by_id.get(&podcast_id)
    }

    /// The playlist ids a podcast auto-adds new episodes to (empty when unknown
    /// or none configured).
    pub fn auto_playlists(&self, podcast_id: i32) -> Vec<i32> {
        self.auto_playlists_by_podcast
            .get(&podcast_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The podcast's auto-add insert-position override (`None` when unknown or
    /// following the server default).
    pub fn auto_playlist_add_to_start(&self, podcast_id: i32) -> Option<bool> {
        self.auto_playlist_add_to_start_by_podcast
            .get(&podcast_id)
            .copied()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): memo slices over this signal need
    /// `PodcastState: PartialEq` to gate re-renders.
    #[test]
    fn podcaststate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<PodcastState>();
    }

    fn pod(id: i32, title: &str) -> PodcastData {
        let ts = Utc::now();
        PodcastData {
            id,
            title: title.to_string(),
            description: String::new(),
            feed_url: String::new(),
            art_url: None,
            author: None,
            polled_at: None,
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: ts,
            updated_at: ts,
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    #[test]
    fn podcast_and_auto_playlists_accessors() {
        let mut s = PodcastState::default();
        s.podcasts_by_id.insert(7, pod(7, "My Show"));
        s.auto_playlists_by_podcast.insert(7, vec![1, 2]);

        assert_eq!(s.podcast(7).map(|p| p.title.as_str()), Some("My Show"));
        assert_eq!(s.podcast(8), None);
        assert_eq!(s.auto_playlists(7), vec![1, 2]);
        assert!(s.auto_playlists(8).is_empty());
    }
}
