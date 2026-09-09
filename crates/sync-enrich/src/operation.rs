use halogen_apiclient::{ApiClient, ApiError};
use halogen_wire::{
    OrderDirection, PlaybackStoreData, PlaylistReorderField, PlaylistUpdateData,
    PodcastConfigUpdateData, PodcastData, PodcastStoreData,
};
use serde::{Deserialize, Serialize};

/// An operation queued locally that must be drained to the server.
///
/// Stored as JSON in the outbox table/object store. The sync worker reads pending
/// ops, sends them to the server via `ApiClient`, and acks them on success.
macro_rules! operation_enum {
    ($name:ident { $($variants:tt)* }) => {
        #[derive(Debug, Clone, Serialize, PartialEq)]
        pub enum $name { $($variants)* }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(remote = "OutboxOp")]
                enum Decode { $($variants)* }
                let mut value = serde_json::Value::deserialize(deserializer)?;
                super::legacy::normalize_operation(&mut value);
                Decode::deserialize(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

operation_enum! {
OutboxOp {
    /// Subscribe = create a podcast from a feed URL, with any directory metadata
    /// known at enqueue time. `#[serde(default)]` keeps pre-metadata ops readable.
    Subscribe {
        feed_url: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        author: Option<String>,
    },
    /// Unsubscribe = delete a podcast.
    Unsubscribe {
        podcast_id: i32,
    },
    MarkPlayed {
        episode_id: i32,
        played: bool,
    },
    SetCursor {
        episode_id: i32,
        cursor: i64,
    },
    /// Bulk-append playlist episodes, except a single position=Some(0) uses positioned add. The server filters
    /// unauthorized IDs and ignores duplicates. Serde aliases/defaults preserve old AddToPlaylistBulk and single
    /// episode_id records so one legacy operation cannot invalidate persisted state.
    #[serde(alias = "AddToPlaylistBulk")]
    AddToPlaylist {
        playlist_id: i32,
        #[serde(default)]
        episode_ids: Vec<i32>,
        /// Insert index: `Some(0)` = front, `None` = append (default). Persisted so
        /// an offline front-of-queue add replays to the server at the same spot it
        /// landed locally.
        #[serde(default)]
        position: Option<i32>,
    },
    /// Remove episodes from a playlist — drained as ONE bulk API call. Non-members
    /// are skipped server-side. As with [`OutboxOp::AddToPlaylist`],
    /// `#[serde(alias)]`/`#[serde(default)]` keep pre-collapse
    /// `RemoveFromPlaylistBulk` and single-remove ops readable.
    #[serde(alias = "RemoveFromPlaylistBulk")]
    RemoveFromPlaylist {
        playlist_id: i32,
        #[serde(default)]
        episode_ids: Vec<i32>,
    },
    /// Reorder an episode within a playlist to target index `to`.
    MoveInPlaylist {
        playlist_id: i32,
        episode_id: i32,
        to: i32,
    },
    /// Reorder a playlist within the user's manual order to target index `to`.
    MovePlaylist {
        playlist_id: i32,
        to: i32,
    },
    /// Smart-reorder a playlist's episodes by a field + direction (bakes the order
    /// into the `Custom` position sequence). The server is authoritative for the
    /// exact order; the client applies a best-effort optimistic order locally.
    ReorderPlaylist {
        playlist_id: i32,
        field: PlaylistReorderField,
        direction: OrderDirection,
    },
    /// Queue offline playlist metadata edits with an existing ID; online edits go directly to show server errors.
    /// Flatten data to preserve the historical playlist_id/name/description/is_default JSON shape.
    UpdatePlaylist {
        playlist_id: i32,
        #[serde(flatten)]
        data: PlaylistUpdateData,
    },
    /// Edit a podcast's download/poll config. Queued only when offline — online edits go direct so the form can
    /// show server errors. Has a real config id (edit, not create), so it's safe to drain later. `data` is
    /// `#[serde(flatten)]`ed so the persisted JSON keeps the historical flat shape — see
    /// [`OutboxOp::UpdatePlaylist`].
    UpdatePodcastConfig {
        config_id: i32,
        #[serde(flatten)]
        data: PodcastConfigUpdateData,
    },
    /// Remove a podcast's config (unlink + delete), reverting it to the server's
    /// global defaults. Targets an existing podcast, so safe to drain later.
    RemovePodcastConfig {
        podcast_id: i32,
    },
    /// Replace the set of playlists a podcast auto-adds new episodes to. The
    /// server filters out unknown ids, so a stale id can't fail the drain.
    /// `add_to_start` is the podcast's insert-position override (`None` =
    /// server default); `#[serde(default)]` keeps pre-field ops readable.
    SetPodcastAutoPlaylists {
        podcast_id: i32,
        playlist_ids: Vec<i32>,
        #[serde(default)]
        add_to_start: Option<bool>,
    },
    /// Ask the server to download episodes' audio (so the client can stream them). Durable/offline-queued
    /// because it needs the server; drained as ONE bulk API call. `#[serde(alias)]`/`#[serde(default)]` keep
    /// pre-collapse `TriggerDownloadBulk` (lossless) and single `TriggerDownload` (the scalar `episode_id`
    /// becomes a one-item list) ops readable on upgrade.
    #[serde(alias = "TriggerDownloadBulk")]
    TriggerDownload {
        #[serde(default)]
        episode_ids: Vec<i32>,
    },
    /// Ask the server to remove its downloaded copies — drained as ONE bulk API
    /// call. Same `#[serde(alias)]`/`#[serde(default)]` back-compat as
    /// [`OutboxOp::TriggerDownload`].
    #[serde(alias = "RemoveServerDownloadBulk")]
    RemoveServerDownload {
        #[serde(default)]
        episode_ids: Vec<i32>,
    },
}

}

mod apply;

#[cfg(test)]
mod tests;
