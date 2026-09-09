use crate::ResponsableData;
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncResource {
    Podcasts,
    Episodes,
    Playbacks,
    Playlists,
    PodcastAutoPlaylists,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct SyncChangesParams {
    #[validate(length(min = 1, max = 80))]
    pub cursor: Option<String>,
    #[validate(range(min = 1, max = 500))]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncChangeData {
    pub sequence: u64,
    pub resource: SyncResource,
    pub resource_id: i32,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncChangesData {
    pub changes: Vec<SyncChangeData>,
    pub next_cursor: String,
    pub has_more: bool,
    pub reset: bool,
}

impl ResponsableData for SyncChangesData {}
