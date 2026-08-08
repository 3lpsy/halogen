use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::meta::{order::Order, pagination::Pagination, response::ResponsableData};
use typeshare::typeshare;

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaybackData {
    pub id: i32,
    pub user_id: i32,
    pub episode_id: i32,
    #[typeshare(serialized_as = "U53")]
    pub cursor: u64,
    /// True once the episode has been listened to the end (or marked played).
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ResponsableData for PlaybackData {}

#[typeshare]
/// Create/update payload for a playback row.
///
/// `user_id` is intentionally absent: the server derives the owner from the
/// authenticated JWT, never from the request body (prevents writing another
/// user's playback). Played/unplayed is carried by `completed`, so `cursor`
/// stays a real position (`>= 0`) instead of a sentinel.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PlaybackStoreData {
    #[validate(range(min = 1, message = "Episode ID must be a valid integer"))]
    pub episode_id: i32,
    #[validate(range(min = 0, max = 36000))]
    #[typeshare(serialized_as = "U53")]
    pub cursor: u64,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PlaybackShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
    #[validate(range(min = 1, message = "User ID must be a valid integer"))]
    pub user_id: Option<i32>,
    #[validate(range(min = 1, message = "Episode ID must be a valid integer"))]
    pub episode_id: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PlaybackListParams {
    #[validate(nested)]
    pub pagination: Option<Pagination>,
    #[validate(nested)]
    pub order: Option<Order>,
    // `user_id` is not accepted: listing is always scoped to the authenticated
    // user (server-side), so a client-supplied filter would be meaningless.
    #[validate(range(min = 1, message = "Episode ID must be a valid integer"))]
    pub episode_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PlaybackDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}
