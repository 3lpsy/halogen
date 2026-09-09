//! Playlists + queue resolution, the worker-owned reactive slice. Its own root signal so a playlist mutation
//! (add/remove/reorder, default resolution) only re-renders playlist/queue consumers, the playlists list, playlist
//! detail, the queue page + its "up next" markers, not every component that reads the main app state. The sync worker
//! is the only writer; the UI reads it reactively (see `halogen_webui_hooks::use_playlists`).

use std::collections::HashMap;

use halogen_wire::{EpisodeData, PlaylistData};

use crate::state::EpisodeState;

/// Distinguish Unknown queue membership from confirmed Absent so UI can wait versus prompt creation. Only the
/// default-playlist endpoint or a definitive local mutation may assert absence; a partial cached pool cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum QueueState {
    /// Not yet resolved — queue actions stay inert (loading), no "no queue" prompt.
    #[default]
    Unknown,
    /// Resolved: the user has no default playlist. Queue actions disabled.
    Absent,
    /// The default playlist's id.
    Present(i32),
}

/// Cached playlists + queue resolution; the worker is the sole writer. `PartialEq` is load-bearing (same reason as on
/// `EpisodeState`): this is read through a signal and sliced by memos, so a derive that silently degraded to "always
/// changed" would defeat the segmentation. The `playliststate_is_partialeq` canary guards it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PlaylistState {
    /// Playlists form a lazily populated partial pool from pages, picks, default lookup, and persisted cache. Never
    /// infer completeness or queue absence by scanning it; use queue/queue_id resolution.
    pub playlists: Vec<PlaylistData>,
    /// Episode ids grouped by playlist_id, in playlist order. Holds only *ids* into
    /// the episode pool (`EpisodeState::episodes_by_id`).
    pub episodes_by_playlist: HashMap<i32, Vec<i32>>,
    /// Resolution of the queue (the default playlist). See [`QueueState`].
    pub queue: QueueState,
}

impl PlaylistState {
    /// The queue (default) playlist id, or `None` when it's `Unknown` or `Absent` —
    /// i.e. there is no actionable queue right now. The single accessor every
    /// queue-targeting action (add/remove-to-queue, the `/queue` page) reads.
    pub fn queue_id(&self) -> Option<i32> {
        match self.queue {
            QueueState::Present(id) => Some(id),
            QueueState::Unknown | QueueState::Absent => None,
        }
    }

    /// The episode that should play next from the **queue** (the default playlist),
    /// given the `current` episode (if any). Shorthand for [`Self::next_up_in`]
    /// with no play context.
    pub fn next_up(&self, current: Option<i32>) -> Option<i32> {
        self.next_up_in(current, None)
    }

    /// Resolve the shared next-up target from known context membership, else queue. Empty/absent lists return None; a
    /// current member advances one position and stops at the tail without fallback. A missing current episode selects
    /// the first item.
    pub fn next_up_in(&self, current: Option<i32>, context: Option<i32>) -> Option<i32> {
        let pid = context
            .filter(|id| self.episodes_by_playlist.contains_key(id))
            .or_else(|| self.queue_id())?;
        let ids = self.episodes_by_playlist.get(&pid)?;
        if let Some(cur) = current
            && let Some(idx) = ids.iter().position(|&id| id == cur)
        {
            // Current is in the playlist: the next one along, or nothing at the tail.
            return ids.get(idx + 1).copied();
        }
        // Not in the playlist (or nothing playing) → start at the head.
        ids.first().copied()
    }

    /// Re-derive `queue` from the locally-known playlists after a mutation. A found
    /// default is authoritative → `Present`. NOT finding one only asserts `Absent`
    /// when `allow_absent` (e.g. an unset/delete that we know emptied the default) —
    /// otherwise a partial list must not be misread as "no queue", so it's left as-is.
    pub fn recompute_queue(&mut self, allow_absent: bool) {
        // Pick the lowest default id, not iteration order: `playlists` is a partial,
        // unordered set, so if two `is_default` entries transiently coexist (an
        // optimistic create + the fetched copy) the queue id must still be
        // deterministic rather than depending on insertion order.
        if let Some(def_id) = self
            .playlists
            .iter()
            .filter(|p| p.is_default)
            .map(|p| p.id)
            .min()
        {
            self.queue = QueueState::Present(def_id);
        } else if allow_absent {
            self.queue = QueueState::Absent;
        }
    }

    /// Clone the episodes for a playlist, in playlist order, resolving ids from the
    /// episode pool (`EpisodeState::episodes_by_id`).
    pub fn playlist_episodes(&self, episodes: &EpisodeState, playlist_id: i32) -> Vec<EpisodeData> {
        Self::resolve(
            &episodes.episodes_by_id,
            self.episodes_by_playlist.get(&playlist_id),
        )
    }

    fn resolve(by_id: &HashMap<i32, EpisodeData>, ids: Option<&Vec<i32>>) -> Vec<EpisodeData> {
        ids.into_iter()
            .flatten()
            .filter_map(|id| by_id.get(id).cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EpisodeState;
    use chrono::Utc;
    use halogen_wire::{DownloadStatus, PlaybackStatus};

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): memo slices over this signal need
    /// `PlaylistState: PartialEq` to gate re-renders.
    #[test]
    fn playliststate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<PlaylistState>();
    }

    fn ep(id: i32) -> EpisodeData {
        let ts = Utc::now();
        EpisodeData {
            id,
            podcast_id: 1,
            title: format!("Episode {id}"),
            description: None,
            content_url: String::new(),
            guid: None,
            art_url: None,
            published_at: Some(ts),
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: None,
            created_at: ts,
            updated_at: ts,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    // ── playlist_episodes ───────────────────────────────────────────────────

    #[test]
    fn playlist_episodes_resolves_in_order_and_drops_missing() {
        let mut episodes = EpisodeState::default();
        episodes.episodes_by_id.insert(10, ep(10));
        episodes.episodes_by_id.insert(20, ep(20));
        episodes.episodes_by_id.insert(30, ep(30));
        let mut s = PlaylistState::default();
        // Playlist order is 30, 99 (missing), 10, 20 → 99 is dropped, order kept.
        s.episodes_by_playlist.insert(5, vec![30, 99, 10, 20]);

        let got: Vec<i32> = s
            .playlist_episodes(&episodes, 5)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(got, vec![30, 10, 20]);
    }

    #[test]
    fn playlist_episodes_empty_for_unknown_playlist() {
        let s = PlaylistState::default();
        assert!(
            s.playlist_episodes(&EpisodeState::default(), 123)
                .is_empty()
        );
    }

    // ── next_up (queue continuation) ────────────────────────────────────────

    #[test]
    fn next_up_follows_the_queue_rules() {
        let mut s = PlaylistState::default();
        // No queue resolved yet → nothing is "up next".
        assert_eq!(s.next_up(Some(10)), None, "no queue → None");

        // Queue = default playlist 5, ordered by position: [30, 10, 20].
        s.queue = QueueState::Present(5);
        s.episodes_by_playlist.insert(5, vec![30, 10, 20]);

        // Current in queue → its position-neighbor.
        assert_eq!(s.next_up(Some(30)), Some(10));
        assert_eq!(s.next_up(Some(10)), Some(20));
        // Tail of the queue → None (auto-advance stops here).
        assert_eq!(s.next_up(Some(20)), None);
        // Not in the queue, or nothing playing → the head of the queue.
        assert_eq!(s.next_up(Some(999)), Some(30));
        assert_eq!(s.next_up(None), Some(30));

        // Empty queue → None even though it's Present.
        s.episodes_by_playlist.insert(5, vec![]);
        assert_eq!(s.next_up(None), None, "empty queue → None");
    }

    // ── next_up_in (play-context continuation) ──────────────────────────────

    #[test]
    fn next_up_in_walks_the_context_playlist() {
        // Queue = default playlist 5 [30, 10, 20]; context playlist 7 [10, 40].
        let mut s = PlaylistState {
            queue: QueueState::Present(5),
            ..Default::default()
        };
        s.episodes_by_playlist.insert(5, vec![30, 10, 20]);
        s.episodes_by_playlist.insert(7, vec![10, 40]);

        // Current in the context playlist → ITS neighbor, not the queue's.
        assert_eq!(s.next_up_in(Some(10), Some(7)), Some(40));
        // Context playlist tail → None (stop; no queue fallback).
        assert_eq!(s.next_up_in(Some(40), Some(7)), None);
        // Current not in the context playlist (or nothing playing) → its head.
        assert_eq!(s.next_up_in(Some(999), Some(7)), Some(10));
        assert_eq!(s.next_up_in(None, Some(7)), Some(10));
        // Context = the queue itself ≡ plain queue semantics.
        assert_eq!(s.next_up_in(Some(10), Some(5)), s.next_up(Some(10)));
        // No context ≡ next_up.
        assert_eq!(s.next_up_in(Some(10), None), Some(20));

        // Empty context playlist → None (nothing to continue with).
        s.episodes_by_playlist.insert(7, vec![]);
        assert_eq!(s.next_up_in(Some(10), Some(7)), None);
    }

    #[test]
    fn next_up_in_unknown_context_degrades_to_queue() {
        let mut s = PlaylistState {
            queue: QueueState::Present(5),
            ..Default::default()
        };
        s.episodes_by_playlist.insert(5, vec![30, 10, 20]);

        // Context playlist with no local membership (deleted/never loaded) →
        // queue semantics.
        assert_eq!(s.next_up_in(Some(10), Some(99)), Some(20));
        // Unknown context AND no queue → None.
        s.queue = QueueState::Absent;
        assert_eq!(s.next_up_in(Some(10), Some(99)), None);
    }
}
