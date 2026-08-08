use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{RequestData, RequestableParams};
use crate::meta::response::{ResponsableData, ResponseData};
use typeshare::typeshare;

#[typeshare]
/// One configured auto-add link: episodes of `podcast_id` are auto-added to
/// `playlist_id` when the RSS poller ingests them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Validate)]
pub struct PodcastAutoPlaylistData {
    pub podcast_id: i32,
    pub playlist_id: i32,
    /// Where auto-added episodes are inserted in this playlist: `Some(true)` =
    /// start, `Some(false)` = end, `None` = the server-wide default
    /// (`subscription_auto_playlist_add_to_start`).
    #[serde(default)]
    pub add_to_start: Option<bool>,
}

impl ResponsableData for PodcastAutoPlaylistData {}

#[typeshare]
/// Body for `PUT /podcasts/{id}/auto-playlists`: the full set of playlists the
/// podcast should auto-add to. The server replaces the existing set with this
/// one (idempotent — handles both first-time create and later edits).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PodcastAutoPlaylistSetData {
    pub playlist_ids: Vec<i32>,
    /// Per-podcast insert-position override stamped on every link: `Some(true)`
    /// = start of the playlists, `Some(false)` = end, `None` = follow the
    /// server-wide default. Older clients omit it → `None` (server default).
    #[serde(default)]
    pub add_to_start: Option<bool>,
}

impl<P: RequestableParams> From<PodcastAutoPlaylistSetData>
    for RequestData<PodcastAutoPlaylistSetData, P>
{
    fn from(data: PodcastAutoPlaylistSetData) -> Self {
        RequestData::from_data(data)
    }
}

impl From<PodcastAutoPlaylistData> for ResponseData<PodcastAutoPlaylistData> {
    fn from(data: PodcastAutoPlaylistData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}
