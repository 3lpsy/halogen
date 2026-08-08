use serde::{Deserialize, Serialize};
use typeshare::typeshare;
use validator::Validate;

use crate::meta::{
    includes::{HasIncludes, Includable},
    order::{Order, OrderDirection},
    pagination::Pagination,
    response::ResponsableData,
};

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Validate)]
pub struct PlaylistData {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    /// Manual order within the user's playlists (0-based). `#[serde(default)]` so
    /// playlist JSON cached before this field existed still deserializes.
    #[serde(default)]
    pub position: i32,
    /// When an episode is removed from this playlist, the server deletes its
    /// server-side download file — only once the episode belongs to no other
    /// playlist. `#[serde(default)]` for JSON cached before the field existed.
    #[serde(default)]
    pub on_remove_delete_file_server: bool,
    /// When the client removes an episode from this playlist, the removing device
    /// also deletes its local media copy. `#[serde(default)]` for old cached JSON.
    #[serde(default)]
    pub on_remove_delete_file_client: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Ordered episode ids (pivot `position`), populated by the `EpisodeIds`
    /// include. `None` = not loaded; `Some([])` = loaded but empty.
    #[serde(default)]
    pub episode_ids: Option<Vec<i32>>,
    /// Raw `episode_playlist` pivot rows, populated by the `EpisodePlaylist`
    /// include.
    #[serde(default)]
    pub episode_playlist: Option<Vec<super::EpisodePlaylistData>>,
}

impl ResponsableData for PlaylistData {}

#[typeshare]
/// Response for `GET /playlists/default`. A wrapper (rather than a bare
/// `Option<PlaylistData>`) so "no queue exists" is an unambiguous `200` with
/// `playlist: null` — distinct from the envelope's `data: null` (which the client
/// treats as a missing-data error).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DefaultPlaylistData {
    /// The default ("queue") playlist, or `None` when the user has no default yet.
    pub playlist: Option<PlaylistData>,
}

impl ResponsableData for DefaultPlaylistData {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PlaylistShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: Option<i32>,
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<PlaylistInclude>>,
}

impl HasIncludes<PlaylistInclude> for PlaylistShowParams {
    fn includes(&mut self) -> &mut Option<Vec<PlaylistInclude>> {
        &mut self.includes
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PlaylistDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PlaylistListParams {
    #[validate(nested)]
    pub pagination: Option<Pagination>,
    #[validate(nested)]
    pub order: Option<Order>,
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<PlaylistInclude>>,
}

impl HasIncludes<PlaylistInclude> for PlaylistListParams {
    fn includes(&mut self) -> &mut Option<Vec<PlaylistInclude>> {
        &mut self.includes
    }
}

/// Body for the playlist reorder endpoint: move a playlist to target index `to`
/// within the user's manual order. The server re-inserts at `to` and rewrites
/// every playlist's `position` 0..n. Mirrors `EpisodePlaylistMoveData`.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PlaylistMoveData {
    pub to: i32,
}

#[typeshare]
/// The field a smart reorder sorts a playlist's episodes by. The chosen order is
/// baked into the `episode_playlist.position` ("Custom") sequence; the user can
/// still hand-tweak afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistReorderField {
    /// Episode `published_at` (nulls last).
    Published,
    /// Episode title, case-insensitive.
    Title,
    /// Episode `duration_secs` (nulls last).
    Duration,
    /// When the episode was added to THIS playlist (`episode_playlist.created_at`).
    Added,
}

impl PlaylistReorderField {
    /// Every variant, in menu order (drives the reorder-page dropdown).
    pub const ALL: [PlaylistReorderField; 4] = [
        PlaylistReorderField::Published,
        PlaylistReorderField::Title,
        PlaylistReorderField::Duration,
        PlaylistReorderField::Added,
    ];

    /// Stable identifier (matches the serde `snake_case` token) for `<select>`
    /// values.
    pub fn as_str(&self) -> &'static str {
        match self {
            PlaylistReorderField::Published => "published",
            PlaylistReorderField::Title => "title",
            PlaylistReorderField::Duration => "duration",
            PlaylistReorderField::Added => "added",
        }
    }

    /// Parse a token back to a field; falls back to `Published`.
    pub fn from_str_or_default(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|f| f.as_str() == s)
            .unwrap_or(PlaylistReorderField::Published)
    }

    /// Human label for the reorder-page dropdown.
    pub fn label(&self) -> &'static str {
        match self {
            PlaylistReorderField::Published => "Published date",
            PlaylistReorderField::Title => "Title",
            PlaylistReorderField::Duration => "Duration",
            PlaylistReorderField::Added => "Date added to playlist",
        }
    }
}

#[typeshare]
/// Body for the smart-reorder endpoint: rewrite the playlist's
/// `episode_playlist.position` so members are ordered by `field` in `direction`.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PlaylistReorderData {
    pub field: PlaylistReorderField,
    #[serde(default)]
    pub direction: OrderDirection,
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
pub struct PlaylistStoreData {
    #[validate(length(min = 1, message = "Playlist name is required"))]
    pub name: String,
    pub description: Option<String>,
    /// Set `true` to make the new playlist the default queue. The server clears
    /// the caller's existing default in the same transaction (a DB partial-unique
    /// index enforces a single default per user). Absent/`false` = a normal playlist.
    #[serde(default)]
    pub is_default: Option<bool>,
    /// Delete the server-side download file when an episode is removed (and no
    /// other playlist still holds it). Absent = `false`.
    #[serde(default)]
    pub on_remove_delete_file_server: Option<bool>,
    /// Removing device deletes its local media copy on episode removal. Absent = `false`.
    #[serde(default)]
    pub on_remove_delete_file_client: Option<bool>,
}

#[typeshare]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Validate)]
pub struct PlaylistUpdateData {
    #[validate(length(min = 1, message = "Playlist name is required"))]
    pub name: Option<String>,
    pub description: Option<String>,
    /// Set `true` to make this the default queue. The server clears the caller's
    /// existing default in the same transaction (a DB partial-unique index enforces
    /// a single default per user). Absent in a request = leave unchanged.
    #[serde(default)]
    pub is_default: Option<bool>,
    /// Delete the server-side download file when an episode is removed (and no
    /// other playlist still holds it). Absent = leave unchanged.
    #[serde(default)]
    pub on_remove_delete_file_server: Option<bool>,
    /// Removing device deletes its local media copy on episode removal.
    /// Absent = leave unchanged.
    #[serde(default)]
    pub on_remove_delete_file_client: Option<bool>,
}

impl PlaylistUpdateData {
    /// Merge this update onto an existing playlist: a `Some` field is applied, a
    /// `None` field leaves the target unchanged. The single source of truth for the
    /// update's field semantics — the client's optimistic updater
    /// (`SyncService::update_playlist_locally`) applies it so an offline edit mirrors
    /// exactly what the server will persist.
    pub fn apply_to(&self, pl: &mut PlaylistData) {
        if let Some(name) = &self.name {
            pl.name = name.clone();
        }
        if let Some(description) = &self.description {
            pl.description = Some(description.clone());
        }
        if let Some(is_default) = self.is_default {
            pl.is_default = is_default;
        }
        if let Some(v) = self.on_remove_delete_file_server {
            pl.on_remove_delete_file_server = v;
        }
        if let Some(v) = self.on_remove_delete_file_client {
            pl.on_remove_delete_file_client = v;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PlaylistInclude {
    /// Full episode bodies (legacy; the client now uses `EpisodeIds` + the pool).
    #[default]
    Episodes,
    /// Ordered episode ids from the `episode_playlist` pivot (by `position`).
    EpisodeIds,
    /// Raw `episode_playlist` pivot rows.
    EpisodePlaylist,
}

impl Includable for PlaylistInclude {}

#[derive(Clone, Debug, Default, Validate, Serialize, Deserialize)]
pub struct DefaultPlaylistParams {
    #[validate(nested)]
    pub pagination: Option<Pagination>,
    #[validate(nested)]
    pub order: Option<Order>,
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<PlaylistInclude>>,
}

impl HasIncludes<PlaylistInclude> for DefaultPlaylistParams {
    fn includes(&mut self) -> &mut Option<Vec<PlaylistInclude>> {
        &mut self.includes
    }
}
