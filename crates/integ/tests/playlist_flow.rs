//! Playlist journey — create a playlist, add/remove episodes, and walk the full
//! CRUD (get/update/delete) plus the `EpisodeIds` membership projection the sync
//! worker uses. Driven through the `ApiClient`, asserting membership + order.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(playlist_flow)'`

use halogen_integ::*;
use halogen_wire::{
    DefaultListParams, Pagination, PlaylistInclude, PlaylistStoreData, PlaylistUpdateData,
};

fn lists_with_ids() -> DefaultListParams<PlaylistInclude> {
    DefaultListParams {
        pagination: Some(Pagination { page: 0, size: 200 }),
        includes: Some(vec![PlaylistInclude::EpisodeIds]),
        ..Default::default()
    }
}

/// The ordered `EpisodeIds` projection for one playlist (the sync-worker path).
async fn episode_ids(client: &halogen_api::ApiClient, playlist_id: i32) -> Vec<i32> {
    client
        .list_playlists(lists_with_ids())
        .await
        .expect("lists")
        .data
        .iter()
        .find(|p| p.id == playlist_id)
        .expect("playlist present")
        .episode_ids
        .clone()
        .unwrap_or_default()
}

#[tokio::test]
async fn playlist_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    // A queue (default playlist) must exist before non-default playlists can be
    // created (the first playlist must be the queue).
    app.seed_playlist("Queue", true).await;

    // Arrange: ingest a podcast with several episodes.
    let (podcast_id, _first) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let eps = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("published_at", halogen_wire::OrderDirection::Desc)),
            vec![],
            Some(halogen_wire::FilterParams {
                podcast_id: Some(podcast_id),
                ..Default::default()
            }),
        ))
        .await
        .expect("episodes")
        .data;
    assert!(eps.len() >= 3, "feed should have >= 3 episodes to order");
    let (e0, e1, e2) = (eps[0].id, eps[1].id, eps[2].id);

    // 1) Create a playlist; the returned row echoes the name.
    let pl = client
        .create_playlist(PlaylistStoreData {
            name: "Favourites".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create");
    assert_eq!(pl.name, "Favourites");
    let playlist_id = pl.id;

    // 2) It starts empty.
    let body = client
        .list_playlist_episodes(playlist_id, ep_page(0, 200))
        .await
        .expect("list eps");
    assert!(body.data.is_empty(), "new playlist is empty");

    // 3) Add episodes in a deliberate order (e2, e0, e1).
    for &id in &[e2, e0, e1] {
        client
            .add_episode(playlist_id, id, None)
            .await
            .expect("add");
    }

    // 4) All three are members. `GET /playlists/{id}/episodes` returns them in
    //    the default episode order (published-desc), not pivot order — so assert
    //    the membership *set* here; position order is checked via EpisodeIds next.
    let body = client
        .list_playlist_episodes(playlist_id, ep_page(0, 200))
        .await
        .expect("list eps");
    let mut got: Vec<i32> = body.data.iter().map(|e| e.id).collect();
    got.sort_unstable();
    let mut want = vec![e0, e1, e2];
    want.sort_unstable();
    assert_eq!(got, want, "all added episodes are members");

    // 5) The `EpisodeIds` projection (the sync worker path) preserves the pivot
    //    *position* order we inserted in (e2, e0, e1).
    assert_eq!(
        episode_ids(&client, playlist_id).await,
        vec![e2, e0, e1],
        "EpisodeIds preserve pivot position order"
    );

    // 6) get_playlist round-trips by id.
    let got = client.get_playlist(playlist_id).await.expect("get");
    assert_eq!(got.id, playlist_id);
    assert_eq!(got.name, "Favourites");

    // 7) Remove the middle-inserted one; it's gone from the membership and the
    //    EpisodeIds order closes up to (e2, e1).
    client
        .remove_episode(playlist_id, e0)
        .await
        .expect("remove");
    let body = client
        .list_playlist_episodes(playlist_id, ep_page(0, 200))
        .await
        .expect("list eps");
    let mut got: Vec<i32> = body.data.iter().map(|e| e.id).collect();
    got.sort_unstable();
    let mut want = vec![e1, e2];
    want.sort_unstable();
    assert_eq!(got, want, "removed episode is gone from membership");
    assert_eq!(
        episode_ids(&client, playlist_id).await,
        vec![e2, e1],
        "EpisodeIds order closes up after removal"
    );

    // 8) Rename via update; the change persists.
    let updated = client
        .update_playlist(
            playlist_id,
            PlaylistUpdateData {
                name: Some("Renamed".into()),
                description: None,
                is_default: None,
                ..Default::default()
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.name, "Renamed");
    assert_eq!(
        client.get_playlist(playlist_id).await.expect("re-get").name,
        "Renamed"
    );

    // 9) Delete; get-by-id then 404s.
    client.delete_playlist(playlist_id).await.expect("delete");
    let err = client.get_playlist(playlist_id).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "deleted playlist is gone");
}

/// Reorder within a playlist via `POST /playlists/{id}/episodes/{episode_id}/move`.
/// The server pulls the episode out and re-inserts it at the target index, then
/// rewrites every position to 0..n — so the `EpisodeIds` projection reflects the
/// new order. Out-of-range targets clamp to the last slot, an unchanged target is
/// a no-op, and moving a non-member episode is a 404.
#[tokio::test]
async fn playlist_move_reorders_positions() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    // A queue (default playlist) must exist before non-default creates.
    app.seed_playlist("Queue", true).await;

    let (podcast_id, _first) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let eps = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("published_at", halogen_wire::OrderDirection::Desc)),
            vec![],
            Some(halogen_wire::FilterParams {
                podcast_id: Some(podcast_id),
                ..Default::default()
            }),
        ))
        .await
        .expect("episodes")
        .data;
    assert!(eps.len() >= 3, "feed should have >= 3 episodes to reorder");
    let (e0, e1, e2) = (eps[0].id, eps[1].id, eps[2].id);

    let pl = client
        .create_playlist(PlaylistStoreData {
            name: "Reorder".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create");
    let pid = pl.id;

    // Insert in order → positions [e0@0, e1@1, e2@2].
    for &id in &[e0, e1, e2] {
        client.add_episode(pid, id, None).await.expect("add");
    }
    assert_eq!(
        episode_ids(&client, pid).await,
        vec![e0, e1, e2],
        "membership starts in insertion order"
    );

    // Move e2 (index 2) to the front (index 0) → [e2, e0, e1].
    client
        .move_episode(pid, e2, 0)
        .await
        .expect("move to front");
    assert_eq!(
        episode_ids(&client, pid).await,
        vec![e2, e0, e1],
        "move-to-front rewrites positions"
    );

    // Move e2 (now index 0) down one (index 1) → [e0, e2, e1].
    client
        .move_episode(pid, e2, 1)
        .await
        .expect("move down one");
    assert_eq!(
        episode_ids(&client, pid).await,
        vec![e0, e2, e1],
        "move down by one swaps with the next item"
    );

    // Out-of-range target clamps to the last slot → move e0 to 99 → [e2, e1, e0].
    client.move_episode(pid, e0, 99).await.expect("move clamps");
    assert_eq!(
        episode_ids(&client, pid).await,
        vec![e2, e1, e0],
        "out-of-range target clamps to the last slot"
    );

    // No-op: moving e2 to its current index (0) leaves order unchanged.
    client.move_episode(pid, e2, 0).await.expect("no-op move");
    assert_eq!(
        episode_ids(&client, pid).await,
        vec![e2, e1, e0],
        "moving to the current index is a no-op"
    );

    // Moving an episode that isn't a member → 404.
    let err = client.move_episode(pid, 999_999, 0).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "moving a non-member episode is 404");
}

/// Playlist edge cases: validation 400 (empty name), 404s (get/update missing
/// playlist, remove a non-member episode), and idempotent duplicate add — a
/// repeated add returns the existing membership row (no second row), backed by a
/// unique (episode_id, playlist_id) index.
#[tokio::test]
async fn playlist_edge_cases() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    // A queue (default playlist) must exist before non-default creates.
    app.seed_playlist("Queue", true).await;

    // Empty name → 400.
    let err = client
        .create_playlist(PlaylistStoreData {
            name: "".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 400, "empty playlist name is a 400");

    // GET / UPDATE a never-existing playlist → 404.
    let err = client.get_playlist(999_999).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "missing playlist get is 404");
    let err = client
        .update_playlist(
            999_999,
            PlaylistUpdateData {
                name: Some("ghost".into()),
                description: None,
                is_default: None,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing playlist update is 404");

    // Arrange a real playlist + episode.
    let podcast_id = app.seed_podcast("PL", "https://feed.test/pl").await;
    let episode_id = app
        .seed_episode(
            podcast_id,
            "Ep",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;
    let other_episode = app
        .seed_episode(
            podcast_id,
            "Other",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;
    let pl = client
        .create_playlist(PlaylistStoreData {
            name: "List".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create");

    // Remove an episode that is NOT a member → 404.
    let err = client
        .remove_episode(pl.id, other_episode)
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "removing a non-member episode is 404");

    // Add succeeds; a DUPLICATE add is idempotent — it returns the existing
    // membership row (no second row) thanks to the unique (episode_id, playlist_id)
    // index.
    client
        .add_episode(pl.id, episode_id, None)
        .await
        .expect("add");
    client
        .add_episode(pl.id, episode_id, None)
        .await
        .expect("duplicate add is idempotent");

    // Add to a NONEXISTENT playlist → 404 (foreign keys are enforced and the
    // ownership guard rejects a missing playlist before the insert).
    let err = client
        .add_episode(999_999, episode_id, None)
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "add to nonexistent playlist is 404");

    // Add a NONEXISTENT episode → 404 (episode existence is checked; the FK is
    // enforced).
    let err = client.add_episode(pl.id, 999_999, None).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "add nonexistent episode is 404");
}

/// Creating a playlist with `is_default: true` makes it the default and clears any
/// previous default in the same transaction (same invariant as promoting via
/// update — never more than one `is_default` row).
#[tokio::test]
async fn creating_as_default_unsets_previous() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let old_default = app.seed_playlist("Queue", true).await;

    // Create a brand-new playlist directly as the default.
    let created = client
        .create_playlist(PlaylistStoreData {
            name: "New Default".into(),
            description: Some("made default on create".into()),
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .expect("create as default");
    assert!(created.is_default, "created playlist should be the default");

    // The previously-default playlist is no longer default.
    assert!(
        !client
            .get_playlist(old_default)
            .await
            .expect("get old")
            .is_default,
        "previous default should have been cleared on create"
    );

    // A plain create (is_default omitted) is not the default.
    let plain = client
        .create_playlist(PlaylistStoreData {
            name: "Plain".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create plain");
    assert!(
        !plain.is_default,
        "omitting is_default creates a normal playlist"
    );
}

/// Promoting a playlist to the default queue clears the previous default — there
/// is never more than one `is_default` row (also enforced by a DB partial unique
/// index; this proves the API clears the old default in the same transaction so
/// the write doesn't trip it).
#[tokio::test]
async fn promoting_to_default_unsets_previous() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let old_default = app.seed_playlist("Queue", true).await;
    let other = app.seed_playlist("Listen Later", false).await;

    // Promote `other`; the server must unset `old_default` in the same txn.
    client
        .update_playlist(
            other,
            PlaylistUpdateData {
                name: None,
                description: None,
                is_default: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("promote to default");

    assert!(
        client
            .get_playlist(other)
            .await
            .expect("get other")
            .is_default,
        "promoted playlist should be the default"
    );
    assert!(
        !client
            .get_playlist(old_default)
            .await
            .expect("get old")
            .is_default,
        "previous default should have been cleared"
    );
}

/// The delete-on-remove cleanup flags: they default false, round-trip through
/// update, and with the server flag set, removing an episode's LAST playlist
/// membership deletes its server download (an episode still held by another
/// playlist is spared).
#[tokio::test]
async fn delete_on_remove_flags_flow() {
    use halogen_wire::DownloadStatus;

    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    app.seed_playlist("Queue", true).await;

    let (podcast_id, _first) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let e0 = app
        .seed_episode(
            podcast_id,
            "Cleanup Sole",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;
    let e1 = app
        .seed_episode(
            podcast_id,
            "Cleanup Shared",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;

    // 1) Flags default to false; an update round-trips them on.
    let pl = client
        .create_playlist(PlaylistStoreData {
            name: "Cleanup".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create");
    assert!(
        !pl.on_remove_delete_file_server && !pl.on_remove_delete_file_client,
        "cleanup flags default to false"
    );
    let updated = client
        .update_playlist(
            pl.id,
            PlaylistUpdateData {
                on_remove_delete_file_server: Some(true),
                on_remove_delete_file_client: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("update flags");
    assert!(
        updated.on_remove_delete_file_server && updated.on_remove_delete_file_client,
        "cleanup flags round-trip through update"
    );

    // 2) e1 is also a member of a second (unflagged) playlist; both episodes are
    //    members of the flagged one and have staged server downloads.
    let other = client
        .create_playlist(PlaylistStoreData {
            name: "Keeper".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create keeper");
    client.add_episode(other.id, e1, None).await.expect("add");
    for &id in &[e0, e1] {
        client.add_episode(pl.id, id, None).await.expect("add");
    }
    let path0 = app.stage_downloaded_audio(e0, b"AUDIO-0").await;
    let path1 = app.stage_downloaded_audio(e1, b"AUDIO-1").await;

    // 3) Bulk-remove both from the flagged playlist (the sync client's path).
    client
        .remove_episodes_bulk(pl.id, vec![e0, e1])
        .await
        .expect("bulk remove");

    // e0 lost its last membership → file gone, status reset.
    assert!(!path0.exists(), "sole-membership episode's file is deleted");
    assert_eq!(
        client
            .get_episode(e0, &[])
            .await
            .expect("get e0")
            .download_status,
        DownloadStatus::NotDownloaded,
        "sole-membership episode resets to NotDownloaded"
    );
    // e1 is still in the other playlist → spared.
    assert!(path1.exists(), "shared episode's file survives");
    assert_eq!(
        client
            .get_episode(e1, &[])
            .await
            .expect("get e1")
            .download_status,
        DownloadStatus::Downloaded,
        "shared episode stays Downloaded"
    );
}
