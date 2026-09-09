//! Derive shared download, queue, playback, and play-gating state for list rows and episode details. Per-row
//! swipe-progress fallbacks and next-up markers remain with callers; common formulas feed EpisodeMenuArgs.

use halogen_wire::DownloadStatus;

use crate::episode_menu::EpisodeMenuArgs;
use halogen_webui_app_state::state::{ClientDownloadState, EpisodeState};
use halogen_webui_app_state::{ConnectionState, DownloadState, PlaylistState};
use halogen_webui_config::PlaybackPreference;
use halogen_webui_player::{PlaybackState, PlayingIdentity};

/// The live download / queue / playback state for one episode, resolved from
/// `EpisodeState` (+ the player's `now_playing`) the same way on every surface that
/// renders an episode. Built by [`resolve_episode_row_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpisodeRowState {
    /// The episode this state describes.
    pub episode_id: i32,
    /// Bytes are stored on THIS device (always playable, offline included).
    pub downloaded_on_device: bool,
    /// A device byte-pull is in flight.
    pub device_downloading: bool,
    /// Live device-download percent (0–100), or `None` before the first byte /
    /// when not downloading.
    pub device_download_progress: Option<u8>,
    /// The server holds the file (so a device download just pulls its copy).
    pub server_downloaded: bool,
    /// The server is mid-fetch of its copy.
    pub server_downloading: bool,
    /// Live server-download percent (0–100), or `None` before the tracker
    /// reports / when not downloading.
    pub server_download_progress: Option<u8>,
    /// The queue (default playlist) id, if one exists.
    pub queue_id: Option<i32>,
    /// Whether this episode is in the queue.
    pub in_queue: bool,
    /// The player is currently on this episode.
    pub is_current: bool,
    /// The player is on this episode AND playing.
    pub is_playing: bool,
    /// The player is on this episode AND spinning up (Preparing/Loading).
    pub is_preparing: bool,
    /// We're offline (no streaming, no server fetch).
    pub is_offline: bool,
    /// Play is unavailable: no device copy, and (offline OR the server lacks the
    /// file). Local-first rule — never playable from just the RSS feed.
    pub play_disabled: bool,
}

impl EpisodeRowState {
    /// Resolve every shared field for `episode_id` from the live pool + player state.
    /// `is_current`/`is_playing`/`is_preparing` come from the player's `playing` identity (the position-free projection
    /// of `now_playing`, so a caller can subscribe to this without re-running on every position tick); the rest from
    /// `app_state`. This is the single copy of the formulas the list item and the detail page used to maintain by hand.
    pub fn resolve(
        app_state: &EpisodeState,
        playlists: &PlaylistState,
        downloads: &DownloadState,
        connection: &ConnectionState,
        playing: &Option<PlayingIdentity>,
        episode_id: i32,
    ) -> Self {
        // Server download status, read LIVE from the pool (server-download
        // commands update the pool optimistically + publish; the captured list
        // prop goes stale). Falls back to absent when the episode isn't pooled.
        let server_status = app_state
            .episode(episode_id)
            .map(|e| e.download_status.clone());
        let server_downloading = server_status.as_ref() == Some(&DownloadStatus::Downloading);
        let server_downloaded = server_status.as_ref() == Some(&DownloadStatus::Downloaded);

        // Device-download state + live percent from the worker-owned download slice.
        let device = downloads.device_state(episode_id);
        let downloaded_on_device = device == Some(ClientDownloadState::Downloaded);
        let device_downloading = device == Some(ClientDownloadState::Downloading);
        let device_download_progress = if device_downloading {
            downloads.download_pct(episode_id)
        } else {
            None
        };
        let server_download_progress = if server_downloading {
            downloads.server_download_pct(episode_id)
        } else {
            None
        };

        // Queue (default playlist) membership.
        let queue_id = playlists.queue_id();
        let in_queue = match queue_id {
            Some(qid) => playlists
                .episodes_by_playlist
                .get(&qid)
                .map(|ids| ids.contains(&episode_id))
                .unwrap_or(false),
            None => false,
        };

        // Player flags for THIS episode.
        let n = playing.as_ref().filter(|n| n.episode_id == episode_id);
        let is_current = n.is_some();
        let is_playing = n
            .map(|n| n.state == PlaybackState::Playing)
            .unwrap_or(false);
        let is_preparing = n
            .map(|n| matches!(n.state, PlaybackState::Preparing | PlaybackState::Loading))
            .unwrap_or(false);

        // Play gating (local-first): a device copy always plays; otherwise the
        // server must hold the file AND we must be online.
        let is_offline = connection.is_offline();
        let play_disabled = !downloaded_on_device && !(server_downloaded && !is_offline);

        Self {
            episode_id,
            downloaded_on_device,
            device_downloading,
            device_download_progress,
            server_downloaded,
            server_downloading,
            server_download_progress,
            queue_id,
            in_queue,
            is_current,
            is_playing,
            is_preparing,
            is_offline,
            play_disabled,
        }
    }
}

/// Resolve the shared live row-state for `episode_id`. Thin free-function alias
/// of [`EpisodeRowState::resolve`] matching the builder phrasing the call sites
/// read at.
pub fn resolve_episode_row_state(
    app_state: &EpisodeState,
    playlists: &PlaylistState,
    downloads: &DownloadState,
    connection: &ConnectionState,
    playing: &Option<PlayingIdentity>,
    episode_id: i32,
) -> EpisodeRowState {
    EpisodeRowState::resolve(
        app_state, playlists, downloads, connection, playing, episode_id,
    )
}

/// The per-list reorder + navigation context an [`EpisodeMenuArgs`] needs on top
/// of the shared [`EpisodeRowState`]. Differs between the list item (a real
/// playlist row with reorder math + "View episode") and the detail page (no
/// playlist context, no "View episode").
#[derive(Clone, Copy, Debug, Default)]
pub struct MenuListContext {
    /// The playlist this row belongs to, when the list is a playlist/queue.
    pub playlist_id: Option<i32>,
    /// Manual (Custom) order is active → the reorder actions are enabled.
    pub reorder_enabled: bool,
    /// This row's index within the rendered list.
    pub position: usize,
    /// Total rendered rows — boundary for Move Down / Move Last.
    pub list_len: usize,
    /// The list is shown in descending Custom order → reorder targets are
    /// mirrored back to stored ascending positions.
    pub reversed: bool,
    /// Full (un-windowed) pool length — the mirror axis when `reversed`.
    pub full_len: usize,
}

impl EpisodeMenuArgs {
    /// Build the ~25-field menu args from the shared row state plus the per-list
    /// context and the playback preference. One constructor so the two call
    /// sites stop maintaining the big struct literal by hand.
    pub fn from_row_state(
        row: &EpisodeRowState,
        podcast_id: i32,
        playback_pref: PlaybackPreference,
        embedded: bool,
        ctx: MenuListContext,
        include_view_episode: bool,
    ) -> Self {
        Self {
            episode_id: row.episode_id,
            podcast_id,
            playback_pref,
            embedded,
            server_downloaded: row.server_downloaded,
            server_downloading: row.server_downloading,
            server_download_progress: row.server_download_progress,
            downloaded_on_device: row.downloaded_on_device,
            device_downloading: row.device_downloading,
            is_offline: row.is_offline,
            queue_id: row.queue_id,
            in_queue: row.in_queue,
            playlist_id: ctx.playlist_id,
            reorder_enabled: ctx.reorder_enabled,
            position: ctx.position,
            list_len: ctx.list_len,
            reversed: ctx.reversed,
            full_len: ctx.full_len,
            include_view_episode,
        }
    }
}
