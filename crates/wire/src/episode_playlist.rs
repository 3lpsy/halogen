use serde::{Deserialize, Serialize};

use crate::meta::response::{ResponsableData, ResponseData};
use typeshare::typeshare;
use validator::Validate;

#[typeshare]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Validate)]
pub struct EpisodePlaylistData {
    pub episode_id: i32,
    pub playlist_id: i32,
    pub position: i32,
}

impl ResponsableData for EpisodePlaylistData {}

#[typeshare]
/// Body for adding an episode to a playlist (POST
/// `/playlists/{playlist_id}/episodes/{episode_id}`). The playlist + episode ids
/// come from the path (the route guard authorizes them), so the body carries only
/// the optional insert position — no duplicated, IDOR-prone ids.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct EpisodePlaylistStoreData {
    /// Where to insert the episode: `Some(0)` = front of the playlist, `Some(n)` =
    /// before the current n-th item (clamped to the end), `None` = append (the
    /// default, so existing callers/bodies that omit it keep appending).
    #[serde(default)]
    pub position: Option<i32>,
}

#[typeshare]
/// Body for the reorder endpoint: move an episode to target index `to` within
/// its playlist. The server re-inserts at `to` and rewrites positions 0..n.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct EpisodePlaylistMoveData {
    pub to: i32,
}

/// Body for the bulk add/remove endpoints (POST / DELETE
/// `/playlists/{playlist_id}/episodes/bulk`). The playlist id comes from the path
/// (the route guard authorizes it); the body carries only the episode ids to add
/// or remove. Mirrors [`crate::episode::EpisodeBulkActionData`] — bulk adds
/// append (no per-episode position), and the server is lenient per id (an
/// unauthorized / already-present / not-a-member id is skipped, not a batch error).
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default)]
pub struct EpisodePlaylistBulkData {
    #[validate(length(
        min = 1,
        max = 500,
        message = "Must include between 1 and 500 episode ids"
    ))]
    pub episode_ids: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct EpisodePlaylistDeleteParams {
    pub episode_id: i32,
    pub playlist_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct EpisodePlaylistStoreParams {
    pub episode_id: i32,
    pub playlist_id: i32,
}

impl From<EpisodePlaylistData> for ResponseData<EpisodePlaylistData> {
    fn from(data: EpisodePlaylistData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}
