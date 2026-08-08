use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::meta::response::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// A single read-only chapter marker on an episode, parsed from the feed's
/// `psc:chapters` (inline) or `podcast:chapters` (external JSON) during sync.
/// There is no CRUD for chapters — they are only ever written by the sync path
/// and read back via the `EpisodeInclude::Chapters` embed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpisodeChapterData {
    pub id: i32,
    pub episode_id: i32,
    pub title: String,
    /// Offset from the start of the episode, in whole seconds.
    pub starts_at_secs: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ResponsableData for EpisodeChapterData {}
