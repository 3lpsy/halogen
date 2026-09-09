use std::collections::HashMap;

use halogen_wire::{DownloadStatus, EpisodeData};

use crate::podcast_state::PodcastState;

/// Sync connectivity status.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SyncStatus {
    /// Cold-start "not yet determined" (before the first probe result lands).
    /// NOT offline: surfaces render cached data / loading, never offline states.
    Unknown,
    /// Connected to server, ready.
    Online,
    /// Cannot reach the server.
    Offline,
    /// Currently syncing data.
    Syncing {
        /// What phase of sync is in progress.
        phase: SyncPhase,
    },
}

impl Default for SyncStatus {
    /// The cold-start status: connectivity is undetermined until the worker's
    /// first probe (manual-offline mirror, WS event, or pull) resolves it.
    fn default() -> Self {
        SyncStatus::Unknown
    }
}

impl SyncStatus {
    /// Whether the worker considers itself offline. The shared predicate behind the
    /// reactive `use_is_offline` slice (in halogen-webui-state) and the non-reactive
    /// `.peek()` reads (submit closures), so both spell "offline" the same way.
    /// `Unknown` (cold start) is deliberately NOT offline.
    pub fn is_offline(&self) -> bool {
        matches!(self, SyncStatus::Offline)
    }

    /// Cold-start "not yet determined" — render as loading, not offline.
    pub fn is_unknown(&self) -> bool {
        matches!(self, SyncStatus::Unknown)
    }
}

/// Describes what part of the sync pipeline is running.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SyncPhase {
    /// Pulling podcast/episode metadata from the server.
    Pull,
    /// Draining the outbox (sending pending ops to the server).
    Drain,
}

/// Model actual reachability and latency separately from sync activity. Navbar overlays persistent manual offline
/// choice, which wins over this measured state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ConnectionHealth {
    /// Not yet determined (cold start, before the first WS/pull result).
    #[default]
    Unknown,
    /// Reachable, round-trip latency within budget (green).
    Online {
        /// Smoothed round-trip latency in ms (last few pongs).
        rtt_ms: u32,
    },
    /// Reachable but sluggish — smoothed RTT over the degraded threshold (yellow).
    /// Still `is_online()`; the UI stays fully functional.
    Degraded {
        /// Smoothed round-trip latency in ms.
        rtt_ms: u32,
    },
    /// Not reachable (red).
    Offline,
}

impl ConnectionHealth {
    /// Whether the server is known reachable (green OR yellow). `false` for
    /// `Offline` and the undetermined cold-start `Unknown`.
    pub fn is_online(self) -> bool {
        matches!(
            self,
            ConnectionHealth::Online { .. } | ConnectionHealth::Degraded { .. }
        )
    }
}

/// Device download state is distinct from server availability. Boot derives committed IDs and resumable partials from
/// media storage; Downloading/Failed flags are transient. Preserve explicit Failed so Preparing watchers can
/// distinguish failure from not-yet-started and offer retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClientDownloadState {
    /// Bytes are being fetched to this device (badge shows a spinner).
    Downloading,
    /// Bytes are stored locally — playable offline.
    Downloaded,
    /// The fetch failed (server fetch failed/timed out, network, or quota).
    Failed,
}

/// Only the worker mutates episode state. Keep PartialEq: ReadSignal prop memoization otherwise dirties the signal on
/// each parent diff, creating subscriber/child render loops. See detail-route render tests.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct EpisodeState {
    /// The single source of truth for episode data, keyed by episode id.
    /// `episodes_by_podcast` (and `PlaylistState::episodes_by_playlist`) hold only
    /// *ids* into this.
    pub episodes_by_id: HashMap<i32, EpisodeData>,
    /// Episode ids grouped by podcast_id (server order).
    pub episodes_by_podcast: HashMap<i32, Vec<i32>>,
}

impl EpisodeState {
    /// Look up an episode by id (O(1)). The canonical accessor — prefer this
    /// over scanning the grouped collections.
    pub fn episode(&self, episode_id: i32) -> Option<&EpisodeData> {
        self.episodes_by_id.get(&episode_id)
    }

    /// Return a server media URL only with downloaded server bytes and configured auth, never the feed enclosure. Web
    /// authenticates with media cookies; native uses the bearer bridge. Local-first playback uses device bytes
    /// normally; this URL serves streaming preferences/actions and missing-byte fallback.
    pub fn audio_url_for_episode(
        &self,
        episode_id: i32,
        server_url: Option<&str>,
        access_token: Option<&str>,
    ) -> Option<String> {
        let ep = self.episode(episode_id)?;
        if ep.download_status != DownloadStatus::Downloaded {
            return None;
        }
        let base = crate::media_url::media_base(server_url)?;
        // Require a session (the media credential is minted alongside this
        // token at login), but the cookie/proxy — not this value — actually
        // authenticates the audio request.
        let _ = access_token?;
        Some(format!("{base}/episodes/{episode_id}/audio"))
    }

    /// Resolve player titles and full/small server artwork from episode and podcast state. Mini/next-up use small art;
    /// full player uses it as an immediate placeholder beneath the full image. Never return feed-origin artwork.
    pub fn episode_display(
        &self,
        podcasts: &PodcastState,
        episode_id: i32,
        server_url: Option<&str>,
    ) -> Option<(String, String, Option<String>, Option<String>)> {
        let ep = self.episode(episode_id)?;
        let podcast = podcasts
            .podcast(ep.podcast_id)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let art = crate::media_url::art_url_for_episode(server_url, ep);
        let art_small = crate::media_url::art_url_for_episode_small(server_url, ep);
        Some((ep.title.clone(), podcast, art, art_small))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use halogen_wire::{PlaybackStatus, PodcastData};

    /// Compile-time canary for the load-bearing derive (see the comment on the `EpisodeState` struct): props
    /// memoization needs `EpisodeState: PartialEq` or `ReadSignal<EpisodeState>` props dirty the signal on every parent
    /// re-render. If a new field ever breaks the derive, this fails to compile with the field named in the error, fix
    /// the field, don't drop the derive.
    #[test]
    fn appstate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<EpisodeState>();
    }

    // ── Fixtures ────────────────────────────────────────────────────────────

    fn ep(id: i32, podcast_id: i32, status: DownloadStatus) -> EpisodeData {
        let ts = Utc::now();
        EpisodeData {
            id,
            podcast_id,
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
            download_status: status,
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

    /// Server base + token live in `ClientConfig` and are passed into the URL
    /// builders, so the tests pass these literals rather than seeding `EpisodeState`.
    const SRV: Option<&str> = Some("https://srv.example");
    const TOK: Option<&str> = Some("tok");

    // ── audio_url_for_episode ───────────────────────────────────────────────────

    #[test]
    fn audio_url_for_episode_none_when_episode_absent() {
        let s = EpisodeState::default();
        assert_eq!(s.audio_url_for_episode(99, SRV, TOK), None);
    }

    #[test]
    fn audio_url_for_episode_none_when_not_downloaded() {
        let mut s = EpisodeState::default();
        s.episodes_by_id
            .insert(1, ep(1, 7, DownloadStatus::NotDownloaded));
        assert_eq!(s.audio_url_for_episode(1, SRV, TOK), None);
        // Downloading on the server is also not playable.
        s.episodes_by_id
            .insert(1, ep(1, 7, DownloadStatus::Downloading));
        assert_eq!(s.audio_url_for_episode(1, SRV, TOK), None);
    }

    #[test]
    fn audio_url_for_episode_none_without_server_url_or_token() {
        let mut s = EpisodeState::default();
        s.episodes_by_id
            .insert(1, ep(1, 7, DownloadStatus::Downloaded));
        assert_eq!(
            s.audio_url_for_episode(1, None, TOK),
            None,
            "no server_url → None"
        );
        assert_eq!(
            s.audio_url_for_episode(1, SRV, None),
            None,
            "no access_token → None"
        );
    }

    // URL-shape note: these tests run natively (renderless `just test-ui`), so
    // they assert the native form — relative `halogen-media` proxy URLs. The
    // wasm arm (absolute `{server}/api/v1/...`) is compile-checked by
    // `just check-all` and exercised by the browser e2e tier.

    #[test]
    fn audio_url_for_episode_some_when_downloaded_with_session() {
        let mut s = EpisodeState::default();
        s.episodes_by_id
            .insert(42, ep(42, 7, DownloadStatus::Downloaded));
        assert_eq!(
            s.audio_url_for_episode(42, Some("https://srv.example/"), TOK),
            Some("/halogen-media/episodes/42/audio".to_string())
        );
    }

    // ── episode_display (art-url builders themselves are tested in `media_url`) ──

    #[test]
    fn episode_display_full_tuple() {
        let mut s = EpisodeState::default();
        s.episodes_by_id
            .insert(1, ep(1, 7, DownloadStatus::Downloaded));
        let mut pods = PodcastState::default();
        pods.podcasts_by_id.insert(7, pod(7, "My Show"));
        let (title, podcast, art, art_small) = s.episode_display(&pods, 1, SRV).unwrap();
        assert_eq!(title, "Episode 1");
        assert_eq!(podcast, "My Show");
        assert_eq!(art, Some("/halogen-media/episodes/1/art".to_string()));
        assert_eq!(
            art_small,
            Some("/halogen-media/episodes/1/art/small".to_string())
        );
    }

    #[test]
    fn episode_display_podcast_title_fallback_empty() {
        // Episode present but its podcast isn't cached → podcast title "".
        let mut s = EpisodeState::default();
        s.episodes_by_id
            .insert(1, ep(1, 7, DownloadStatus::Downloaded));
        let (title, podcast, art, art_small) = s
            .episode_display(&PodcastState::default(), 1, None)
            .unwrap();
        assert_eq!(title, "Episode 1");
        assert_eq!(podcast, "", "missing podcast → empty title fallback");
        assert_eq!(art, None, "no server_url → no art");
        assert_eq!(art_small, None, "no server_url → no small art");
    }

    #[test]
    fn episode_display_none_when_episode_absent() {
        let s = EpisodeState::default();
        assert_eq!(s.episode_display(&PodcastState::default(), 404, SRV), None);
    }
}
