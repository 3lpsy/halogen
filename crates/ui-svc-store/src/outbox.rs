use halogen_api::{ApiClient, ApiError};
use halogen_wire::{
    OrderDirection, PlaybackStoreData, PlaylistReorderField, PlaylistUpdateData,
    PodcastConfigUpdateData, PodcastData, PodcastStoreData,
};
use serde::{Deserialize, Serialize};

/// An operation queued locally that must be drained to the server.
///
/// Stored as JSON in the outbox table/object store. The sync worker reads pending
/// ops, sends them to the server via `ApiClient`, and acks them on success.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutboxOp {
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
    /// Add episodes to a playlist. Drained as ONE bulk API call (append), except a
    /// front-of-queue add (`position = Some(0)`, only ever a single id) which uses
    /// the positioned single-add endpoint. The server filters ids the caller isn't
    /// authorized for and is idempotent on already-present ids.
    ///
    /// `#[serde(alias)]` + `#[serde(default)]` keep ops persisted by the
    /// pre-collapse format readable so an upgrade never fails to deserialize the
    /// outbox (on web the whole array loads at once — one bad op would wipe it):
    /// an old `AddToPlaylistBulk` deserializes here losslessly, and an old
    /// single-add (which carried `episode_id`) lands with empty `episode_ids` and
    /// drains as a harmless no-op.
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
    /// Edit a playlist's metadata (name/description/make-default). Queued only when
    /// offline — online edits go direct so the form can show server errors. Has a
    /// real id (edit, not create), so it's safe to drain later.
    ///
    /// `data` is `#[serde(flatten)]`ed so the persisted JSON keeps the historical
    /// flat shape (`{playlist_id, name, description, is_default}`) — ops queued by
    /// the pre-DTO format deserialize unchanged.
    UpdatePlaylist {
        playlist_id: i32,
        #[serde(flatten)]
        data: PlaylistUpdateData,
    },
    /// Edit a podcast's download/poll config. Queued only when offline — online
    /// edits go direct so the form can show server errors. Has a real config id
    /// (edit, not create), so it's safe to drain later.
    ///
    /// `data` is `#[serde(flatten)]`ed so the persisted JSON keeps the historical
    /// flat shape — see [`OutboxOp::UpdatePlaylist`].
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
    /// Ask the server to download episodes' audio (so the client can stream them).
    /// Durable/offline-queued because it needs the server; drained as ONE bulk API
    /// call. `#[serde(alias)]`/`#[serde(default)]` keep pre-collapse
    /// `TriggerDownloadBulk` (lossless) and single `TriggerDownload` (the scalar
    /// `episode_id` is ignored → empty ids → no-op) ops readable on upgrade.
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

/// Clamp to the DTO's char-count validation bound (validator counts chars).
fn clamp_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl OutboxOp {
    /// Send this queued op to the server. This is the single place that maps an
    /// outbox op to an API call — the worker drains ops and acks on `Ok`.
    /// `Subscribe` returns the created/reused podcast so the worker can merge it
    /// into the pool immediately (the post-drain pull doesn't fetch podcasts).
    pub async fn apply(&self, api: &ApiClient) -> Result<Option<PodcastData>, ApiError> {
        match self {
            OutboxOp::Subscribe {
                feed_url,
                title,
                description,
                author,
            } => {
                // The DTO requires a 1-256 char title; fall back to the feed URL —
                // the RSS ingest heals it to the channel title on first sync.
                let title = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(feed_url);
                api.create_podcast(PodcastStoreData {
                    title: clamp_chars(title, 256),
                    description: description.as_deref().map(|d| clamp_chars(d, 65536)),
                    feed_url: feed_url.clone(),
                    art_url: None,
                    author: author.as_deref().map(|a| clamp_chars(a, 256)),
                    podcast_config_id: None,
                })
                .await
                .map(Some)
            }
            OutboxOp::Unsubscribe { podcast_id } => {
                api.delete_podcast(*podcast_id).await.map(|_| None)
            }
            // Marking played/unplayed resets the position to 0 — deliberately, and
            // in lock-step with the optimistic local change (`set_playback_locally`
            // sets `pb.cursor = 0` for the same command), so a finished/reset
            // episode shows no resume position on either side.
            OutboxOp::MarkPlayed { episode_id, played } => api
                .upsert_playback(PlaybackStoreData {
                    episode_id: *episode_id,
                    cursor: 0,
                    completed: *played,
                })
                .await
                .map(|_| None),
            OutboxOp::SetCursor { episode_id, cursor } => api
                .upsert_playback(PlaybackStoreData {
                    episode_id: *episode_id,
                    // Client-side cursors stay i64 (seek math may transiently go
                    // negative); the wire type is u64 — clamp at the boundary.
                    cursor: u64::try_from(*cursor).unwrap_or(0),
                    completed: false,
                })
                .await
                .map(|_| None),
            OutboxOp::AddToPlaylist {
                playlist_id,
                episode_ids,
                position,
            } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                match position {
                    // Front-of-queue: a positioned insert is only ever a single id,
                    // and the bulk endpoint appends only — so use the positioned
                    // single-add endpoint.
                    Some(pos) => {
                        for episode_id in episode_ids {
                            api.add_episode(*playlist_id, *episode_id, Some(*pos))
                                .await?;
                        }
                        Ok(None)
                    }
                    None => api
                        .add_episodes_bulk(*playlist_id, episode_ids.clone())
                        .await
                        .map(|_| None),
                }
            }
            OutboxOp::RemoveFromPlaylist {
                playlist_id,
                episode_ids,
            } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.remove_episodes_bulk(*playlist_id, episode_ids.clone())
                    .await
                    .map(|_| None)
            }
            OutboxOp::MoveInPlaylist {
                playlist_id,
                episode_id,
                to,
            } => api
                .move_episode(*playlist_id, *episode_id, *to)
                .await
                .map(|_| None),
            OutboxOp::MovePlaylist { playlist_id, to } => {
                api.move_playlist(*playlist_id, *to).await.map(|_| None)
            }
            OutboxOp::ReorderPlaylist {
                playlist_id,
                field,
                direction,
            } => api
                .reorder_playlist(*playlist_id, *field, direction.clone())
                .await
                .map(|_| None),
            OutboxOp::UpdatePlaylist { playlist_id, data } => api
                .update_playlist(*playlist_id, data.clone())
                .await
                .map(|_| None),
            OutboxOp::UpdatePodcastConfig { config_id, data } => api
                .update_podcast_config(*config_id, data.clone())
                .await
                .map(|_| None),
            OutboxOp::RemovePodcastConfig { podcast_id } => api
                .remove_podcast_config_for(*podcast_id)
                .await
                .map(|_| None),
            OutboxOp::SetPodcastAutoPlaylists {
                podcast_id,
                playlist_ids,
                add_to_start,
            } => api
                .set_podcast_auto_playlists(*podcast_id, playlist_ids.clone(), *add_to_start)
                .await
                .map(|_| None),
            OutboxOp::TriggerDownload { episode_ids } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.trigger_download_bulk(episode_ids.clone())
                    .await
                    .map(|_| None)
            }
            OutboxOp::RemoveServerDownload { episode_ids } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.remove_server_download_bulk(episode_ids.clone())
                    .await
                    .map(|_| None)
            }
        }
    }

    /// True for ops that change server-side structure the client can't fully
    /// reproduce locally (new podcast + its episodes), so a pull should follow.
    pub fn needs_pull_after(&self) -> bool {
        matches!(
            self,
            OutboxOp::Subscribe { .. } | OutboxOp::Unsubscribe { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(op: OutboxOp) {
        let json = serde_json::to_string(&op).unwrap();
        let loaded: OutboxOp = serde_json::from_str(&json).unwrap();
        assert_eq!(op, loaded);
    }

    #[test]
    fn outbox_op_roundtrips() {
        roundtrip(OutboxOp::Subscribe {
            feed_url: "https://example.com/feed.xml".into(),
            title: Some("A Show".into()),
            description: None,
            author: Some("Host".into()),
        });
        roundtrip(OutboxOp::Unsubscribe { podcast_id: 3 });
        roundtrip(OutboxOp::MarkPlayed {
            episode_id: 42,
            played: true,
        });
        roundtrip(OutboxOp::SetCursor {
            episode_id: 1,
            cursor: 12345,
        });
        roundtrip(OutboxOp::AddToPlaylist {
            playlist_id: 5,
            episode_ids: vec![10],
            position: Some(0),
        });
        roundtrip(OutboxOp::RemoveFromPlaylist {
            playlist_id: 3,
            episode_ids: vec![7],
        });
        roundtrip(OutboxOp::AddToPlaylist {
            playlist_id: 5,
            episode_ids: vec![10, 11, 12],
            position: None,
        });
        roundtrip(OutboxOp::RemoveFromPlaylist {
            playlist_id: 3,
            episode_ids: vec![7, 8],
        });
        roundtrip(OutboxOp::MoveInPlaylist {
            playlist_id: 3,
            episode_id: 7,
            to: 2,
        });
        roundtrip(OutboxOp::MovePlaylist {
            playlist_id: 3,
            to: 1,
        });
        roundtrip(OutboxOp::ReorderPlaylist {
            playlist_id: 3,
            field: PlaylistReorderField::Published,
            direction: OrderDirection::Desc,
        });
        roundtrip(OutboxOp::UpdatePlaylist {
            playlist_id: 3,
            data: PlaylistUpdateData {
                name: Some("Renamed".into()),
                description: None,
                is_default: Some(true),
                ..Default::default()
            },
        });
        roundtrip(OutboxOp::UpdatePodcastConfig {
            config_id: 4,
            data: PodcastConfigUpdateData {
                poll_interval_seconds: Some(3600),
                max_episodes: Some(50),
                max_concurrent_downloads: None,
                auto_download_enabled: Some(true),
            },
        });
        roundtrip(OutboxOp::RemovePodcastConfig { podcast_id: 8 });
        roundtrip(OutboxOp::SetPodcastAutoPlaylists {
            podcast_id: 8,
            playlist_ids: vec![1, 4, 7],
            add_to_start: Some(true),
        });
        roundtrip(OutboxOp::TriggerDownload {
            episode_ids: vec![9],
        });
        roundtrip(OutboxOp::RemoveServerDownload {
            episode_ids: vec![11],
        });
        roundtrip(OutboxOp::TriggerDownload {
            episode_ids: vec![1, 2, 3],
        });
        roundtrip(OutboxOp::RemoveServerDownload {
            episode_ids: vec![4, 5],
        });
    }

    /// A Subscribe persisted before the metadata fields must still deserialize.
    #[test]
    fn subscribe_pre_metadata_json_still_reads() {
        let loaded: OutboxOp =
            serde_json::from_str(r#"{"Subscribe":{"feed_url":"https://example.com/f.xml"}}"#)
                .unwrap();
        assert_eq!(
            loaded,
            OutboxOp::Subscribe {
                feed_url: "https://example.com/f.xml".into(),
                title: None,
                description: None,
                author: None,
            }
        );
    }

    #[test]
    fn needs_pull_after_only_for_subscribe_unsubscribe() {
        assert!(
            OutboxOp::Subscribe {
                feed_url: "https://example.com/feed.xml".into(),
                title: None,
                description: None,
                author: None,
            }
            .needs_pull_after()
        );
        assert!(OutboxOp::Unsubscribe { podcast_id: 1 }.needs_pull_after());

        // Everything else is locally reproducible → no pull.
        assert!(
            !OutboxOp::MarkPlayed {
                episode_id: 1,
                played: true
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::SetCursor {
                episode_id: 1,
                cursor: 10
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::AddToPlaylist {
                playlist_id: 1,
                episode_ids: vec![2],
                position: None
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::RemoveFromPlaylist {
                playlist_id: 1,
                episode_ids: vec![2]
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::MoveInPlaylist {
                playlist_id: 1,
                episode_id: 2,
                to: 0
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::MovePlaylist {
                playlist_id: 1,
                to: 0
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::TriggerDownload {
                episode_ids: vec![3]
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::RemoveServerDownload {
                episode_ids: vec![3]
            }
            .needs_pull_after()
        );
        assert!(
            !OutboxOp::UpdatePodcastConfig {
                config_id: 4,
                data: PodcastConfigUpdateData {
                    poll_interval_seconds: Some(3600),
                    max_episodes: None,
                    max_concurrent_downloads: None,
                    auto_download_enabled: None,
                },
            }
            .needs_pull_after()
        );
        assert!(!OutboxOp::RemovePodcastConfig { podcast_id: 8 }.needs_pull_after());
        assert!(
            !OutboxOp::SetPodcastAutoPlaylists {
                podcast_id: 8,
                playlist_ids: vec![1, 2],
                add_to_start: None,
            }
            .needs_pull_after()
        );
    }

    #[test]
    fn legacy_playlist_ops_deserialize_after_collapse() {
        // Pre-collapse BULK ops drain losslessly via `#[serde(alias)]` — the old
        // tag maps onto the unified variant with its ids intact.
        let add_bulk: OutboxOp =
            serde_json::from_str(r#"{"AddToPlaylistBulk":{"playlist_id":5,"episode_ids":[1,2]}}"#)
                .unwrap();
        assert_eq!(
            add_bulk,
            OutboxOp::AddToPlaylist {
                playlist_id: 5,
                episode_ids: vec![1, 2],
                position: None,
            }
        );
        let rm_bulk: OutboxOp = serde_json::from_str(
            r#"{"RemoveFromPlaylistBulk":{"playlist_id":3,"episode_ids":[7,8]}}"#,
        )
        .unwrap();
        assert_eq!(
            rm_bulk,
            OutboxOp::RemoveFromPlaylist {
                playlist_id: 3,
                episode_ids: vec![7, 8],
            }
        );

        // Pre-collapse SINGLE ops still DESERIALIZE (the legacy `episode_id` field
        // is ignored) — critical, because web loads the whole outbox array at once
        // and one hard error would wipe every pending op. They land with empty
        // `episode_ids` and drain as a harmless no-op.
        let add_single: OutboxOp = serde_json::from_str(
            r#"{"AddToPlaylist":{"playlist_id":5,"episode_id":10,"position":0}}"#,
        )
        .unwrap();
        assert_eq!(
            add_single,
            OutboxOp::AddToPlaylist {
                playlist_id: 5,
                episode_ids: vec![],
                position: Some(0),
            }
        );
        let rm_single: OutboxOp =
            serde_json::from_str(r#"{"RemoveFromPlaylist":{"playlist_id":3,"episode_id":7}}"#)
                .unwrap();
        assert_eq!(
            rm_single,
            OutboxOp::RemoveFromPlaylist {
                playlist_id: 3,
                episode_ids: vec![],
            }
        );
    }

    #[test]
    fn flattened_update_ops_keep_flat_json() {
        // The DTO is `#[serde(flatten)]`ed, so the persisted JSON stays flat — ops
        // queued by the pre-DTO format (fields at the top level of the variant)
        // deserialize unchanged, and new ops serialize to that same flat shape.
        let pl: OutboxOp = serde_json::from_str(
            r#"{"UpdatePlaylist":{"playlist_id":3,"name":"Renamed","description":null,"is_default":true}}"#,
        )
        .unwrap();
        assert_eq!(
            pl,
            OutboxOp::UpdatePlaylist {
                playlist_id: 3,
                data: PlaylistUpdateData {
                    name: Some("Renamed".into()),
                    description: None,
                    is_default: Some(true),
                    ..Default::default()
                },
            }
        );
        // Round-trip back out to the flat shape (the unset delete-on-remove flags
        // serialize as explicit nulls).
        assert_eq!(
            serde_json::to_value(&pl).unwrap(),
            serde_json::json!({"UpdatePlaylist":{"playlist_id":3,"name":"Renamed","description":null,"is_default":true,"on_remove_delete_file_server":null,"on_remove_delete_file_client":null}})
        );

        let pc: OutboxOp = serde_json::from_str(
            r#"{"UpdatePodcastConfig":{"config_id":4,"poll_interval_seconds":3600,"max_episodes":50,"max_concurrent_downloads":null,"auto_download_enabled":true}}"#,
        )
        .unwrap();
        assert_eq!(
            pc,
            OutboxOp::UpdatePodcastConfig {
                config_id: 4,
                data: PodcastConfigUpdateData {
                    poll_interval_seconds: Some(3600),
                    max_episodes: Some(50),
                    max_concurrent_downloads: None,
                    auto_download_enabled: Some(true),
                },
            }
        );
    }

    #[test]
    fn legacy_download_ops_deserialize_after_collapse() {
        // Old bulk ops drain losslessly via `#[serde(alias)]`.
        let trigger_bulk: OutboxOp =
            serde_json::from_str(r#"{"TriggerDownloadBulk":{"episode_ids":[1,2,3]}}"#).unwrap();
        assert_eq!(
            trigger_bulk,
            OutboxOp::TriggerDownload {
                episode_ids: vec![1, 2, 3],
            }
        );
        let remove_bulk: OutboxOp =
            serde_json::from_str(r#"{"RemoveServerDownloadBulk":{"episode_ids":[4,5]}}"#).unwrap();
        assert_eq!(
            remove_bulk,
            OutboxOp::RemoveServerDownload {
                episode_ids: vec![4, 5],
            }
        );
        // Old single ops carried a scalar `episode_id`; it's ignored → empty ids →
        // no-op on drain, never erroring (which on web would wipe the whole outbox).
        let trigger_single: OutboxOp =
            serde_json::from_str(r#"{"TriggerDownload":{"episode_id":9}}"#).unwrap();
        assert_eq!(
            trigger_single,
            OutboxOp::TriggerDownload {
                episode_ids: vec![]
            }
        );
        let remove_single: OutboxOp =
            serde_json::from_str(r#"{"RemoveServerDownload":{"episode_id":11}}"#).unwrap();
        assert_eq!(
            remove_single,
            OutboxOp::RemoveServerDownload {
                episode_ids: vec![]
            }
        );
    }
}
