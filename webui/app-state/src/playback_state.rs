//! Playback cursors overlay, the worker-owned reactive slice. Its own root signal so a cursor save (seek / mark-played,
//! ~every 10s while playing) or a History page only re-renders playback consumers, the row progress bars + played
//! markers, the History list, not every component that reads the main app state. The sync worker is the only writer;
//! the UI reads it reactively (see `halogen_webui_hooks::use_playbacks`).

use std::collections::HashMap;

use halogen_wire::PlaybackData;

use crate::state::EpisodeState;

/// Per-episode playback cursors; the worker is the sole writer. `PartialEq` is load-bearing (same reason as on
/// `EpisodeState`): this is read through a signal and sliced by memos, so a derive that silently degraded to "always
/// changed" would defeat the segmentation. The `playbackstate_is_partialeq` canary guards it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PlaybackState {
    /// Playback positions keyed by episode_id. NOT a full mirror of the server, it's a just-in-time **overlay**:
    /// locally-written cursors (seek / mark-played) plus whatever's been fetched (episode-page `Playback` includes,
    /// History paging, boot hydrate from the local store). Rows read it overlay-wins over the body's embedded cursor;
    /// History derives its ordered membership from it.
    pub playbacks: HashMap<i32, PlaybackData>,
}

impl PlaybackState {
    /// The caller's resume playback for an episode (overlay-wins): the optimistic overlay (`playbacks`, written locally
    /// on seek / mark-played) wins, else the cursor the server embedded on the cached episode body
    /// (`EpisodeInclude::Playback`), resolved from the episode pool (`EpisodeState`). `None` when neither knows a
    /// position. Single source of truth for resume + progress: playbacks are fetched just-in-time, not bulk-pulled.
    pub fn playback_for(&self, episodes: &EpisodeState, episode_id: i32) -> Option<PlaybackData> {
        if let Some(pb) = self.playbacks.get(&episode_id) {
            return Some(pb.clone());
        }
        episodes
            .episode(episode_id)
            .and_then(|e| e.playback.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EpisodeState;
    use chrono::Utc;
    use halogen_wire::{DownloadStatus, EpisodeData, PlaybackStatus};

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): memo slices over this signal need
    /// `PlaybackState: PartialEq` to gate re-renders.
    #[test]
    fn playbackstate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<PlaybackState>();
    }

    fn ep(id: i32) -> EpisodeData {
        let ts = Utc::now();
        EpisodeData {
            id,
            podcast_id: 7,
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
            download_status: DownloadStatus::Downloaded,
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

    fn pb(episode_id: i32, cursor: u64) -> PlaybackData {
        let ts = Utc::now();
        PlaybackData {
            id: 1,
            user_id: 1,
            episode_id,
            cursor,
            completed: false,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn playback_for_prefers_overlay_then_embedded_then_none() {
        let mut episodes = EpisodeState::default();
        let mut body = ep(1);
        body.playback = Some(pb(1, 30)); // cursor the server embedded on the body
        episodes.episodes_by_id.insert(1, body);

        let mut s = PlaybackState::default();
        // No overlay entry → fall back to the body's embedded cursor.
        assert_eq!(s.playback_for(&episodes, 1).map(|p| p.cursor), Some(30));

        // A local write (overlay) wins over the embedded cursor.
        s.playbacks.insert(1, pb(1, 999));
        assert_eq!(s.playback_for(&episodes, 1).map(|p| p.cursor), Some(999));

        // Unknown episode, no overlay → None.
        assert_eq!(s.playback_for(&episodes, 2), None);
    }
}
