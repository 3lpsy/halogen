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
        serde_json::from_str(r#"{"Subscribe":{"feed_url":"https://example.com/f.xml"}}"#).unwrap();
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
    let rm_bulk: OutboxOp =
        serde_json::from_str(r#"{"RemoveFromPlaylistBulk":{"playlist_id":3,"episode_ids":[7,8]}}"#)
            .unwrap();
    assert_eq!(
        rm_bulk,
        OutboxOp::RemoveFromPlaylist {
            playlist_id: 3,
            episode_ids: vec![7, 8],
        }
    );

    // Legacy scalar IDs retain their original membership intent.
    let add_single: OutboxOp =
        serde_json::from_str(r#"{"AddToPlaylist":{"playlist_id":5,"episode_id":10,"position":0}}"#)
            .unwrap();
    assert_eq!(
        add_single,
        OutboxOp::AddToPlaylist {
            playlist_id: 5,
            episode_ids: vec![10],
            position: Some(0),
        }
    );
    let rm_single: OutboxOp =
        serde_json::from_str(r#"{"RemoveFromPlaylist":{"playlist_id":3,"episode_id":7}}"#).unwrap();
    assert_eq!(
        rm_single,
        OutboxOp::RemoveFromPlaylist {
            playlist_id: 3,
            episode_ids: vec![7],
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
    // Legacy single-download requests remain replayable.
    let trigger_single: OutboxOp =
        serde_json::from_str(r#"{"TriggerDownload":{"episode_id":9}}"#).unwrap();
    assert_eq!(
        trigger_single,
        OutboxOp::TriggerDownload {
            episode_ids: vec![9]
        }
    );
    let remove_single: OutboxOp =
        serde_json::from_str(r#"{"RemoveServerDownload":{"episode_id":11}}"#).unwrap();
    assert_eq!(
        remove_single,
        OutboxOp::RemoveServerDownload {
            episode_ids: vec![11]
        }
    );
}
