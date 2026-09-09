use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};

/// One cache mutation and its delivery operations commit together.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StoreChanges {
    pub check_sync_cursor: bool,
    pub expected_sync_cursor: Option<String>,
    pub require_empty_pending: bool,
    pub sync_cursor: Option<String>,
    pub reset_cache: bool,
    pub auto_playlists: std::collections::BTreeMap<i32, Vec<halogen_wire::PodcastAutoPlaylistData>>,
    pub deleted_auto_playlists: Vec<i32>,
    pub podcasts: Vec<PodcastData>,
    pub episodes: Vec<EpisodeData>,
    pub playlists: Vec<PlaylistData>,
    pub playbacks: Vec<PlaybackData>,
    pub deleted_podcasts: Vec<i32>,
    pub deleted_episodes: Vec<i32>,
    pub deleted_playlists: Vec<i32>,
    pub deleted_playbacks: Vec<i32>,
    pub acknowledged_operations: Vec<u64>,
}
