//! Podcast auto-playlist journey — the set-replace lifecycle of a podcast's
//! auto-add playlists, plus the behavior that motivates the feature: newly
//! polled episodes land in every configured playlist. Driven through the real
//! `ApiClient`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(podcast_auto_playlist_flow)'`

use halogen_integ::*;
use halogen_wire::{OrderDirection, PodcastStoreData};

/// Create a podcast and return its id (no config, no episodes).
async fn make_podcast(client: &halogen_api::ApiClient, feed_url: &str) -> i32 {
    client
        .create_podcast(PodcastStoreData {
            title: "Auto".into(),
            description: None,
            feed_url: feed_url.into(),
            art_url: None,
            author: None,
            podcast_config_id: None,
        })
        .await
        .expect("create podcast")
        .id
}

/// Create a named (non-default) playlist and return its id. Seeded directly via
/// the harness — the API's "first playlist must be the default queue" rule only
/// guards the create endpoint, and these journeys don't need a queue.
async fn make_playlist(app: &TestApp, name: &str) -> i32 {
    app.seed_playlist(name, false).await
}

/// The configured playlist ids for a podcast, sorted for stable assertions.
async fn auto_ids(client: &halogen_api::ApiClient, podcast_id: i32) -> Vec<i32> {
    let mut ids: Vec<i32> = client
        .get_podcast_auto_playlists(podcast_id)
        .await
        .expect("get auto-playlists")
        .into_iter()
        .map(|r| r.playlist_id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Set → get → replace → clear: the full set-replace lifecycle.
#[tokio::test]
async fn auto_playlist_set_get_replace_clear() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let podcast = make_podcast(&client, "https://example.com/auto-feed.xml").await;
    let p1 = make_playlist(&app, "One").await;
    let p2 = make_playlist(&app, "Two").await;

    // Nothing configured yet.
    assert!(
        auto_ids(&client, podcast).await.is_empty(),
        "empty to start"
    );

    // Set both; get echoes them.
    let set = client
        .set_podcast_auto_playlists(podcast, vec![p1, p2], None)
        .await
        .expect("set both");
    assert_eq!(set.len(), 2, "both stored");
    assert_eq!(auto_ids(&client, podcast).await, {
        let mut v = vec![p1, p2];
        v.sort_unstable();
        v
    });

    // Replace with just one — it's a replace, not an append.
    client
        .set_podcast_auto_playlists(podcast, vec![p2], None)
        .await
        .expect("replace");
    assert_eq!(auto_ids(&client, podcast).await, vec![p2]);

    // Clear.
    client
        .set_podcast_auto_playlists(podcast, vec![], None)
        .await
        .expect("clear");
    assert!(auto_ids(&client, podcast).await.is_empty(), "cleared");
}

/// A non-existent playlist id is silently dropped (lenient), not a 500/400.
#[tokio::test]
async fn auto_playlist_unknown_id_is_filtered() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let podcast = make_podcast(&client, "https://example.com/auto-feed-2.xml").await;
    let p1 = make_playlist(&app, "Real").await;

    let set = client
        .set_podcast_auto_playlists(podcast, vec![p1, 999_999], None)
        .await
        .expect("set with one unknown id still succeeds");
    assert_eq!(
        set.iter().map(|r| r.playlist_id).collect::<Vec<_>>(),
        vec![p1],
        "unknown id dropped, real id kept"
    );
}

/// Deleting a playlist cascades: its auto-add links vanish and get still works.
#[tokio::test]
async fn auto_playlist_cascades_on_playlist_delete() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let podcast = make_podcast(&client, "https://example.com/auto-feed-3.xml").await;
    let p1 = make_playlist(&app, "Keep").await;
    let p2 = make_playlist(&app, "Doomed").await;

    client
        .set_podcast_auto_playlists(podcast, vec![p1, p2], None)
        .await
        .expect("set both");

    // Delete one of the linked playlists.
    client.delete_playlist(p2).await.expect("delete playlist");

    // The link to the deleted playlist is gone; the call doesn't fail.
    assert_eq!(
        auto_ids(&client, podcast).await,
        vec![p1],
        "deleted playlist's link cascaded away"
    );
}

/// Setting auto-playlists for a missing podcast is a 404.
#[tokio::test]
async fn auto_playlist_missing_podcast_404() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let err = client
        .set_podcast_auto_playlists(999_999, vec![], None)
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing podcast is 404");
}

/// The core behavior: with a podcast configured to auto-add to a playlist, a
/// poll that ingests new episodes lands every one of them in that playlist.
#[tokio::test]
async fn auto_playlist_rss_poll_adds_new_episodes() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // A podcast pointed at a mocked feed (seeded directly so its feed_url is the
    // wiremock uri), and a target playlist.
    let feed = load_feed("hanselminutes.xml");
    let upstream = mock_feed(&feed).await;
    let podcast_id = app.seed_podcast("Auto Poll", &upstream.uri()).await;
    let playlist_id = make_playlist(&app, "Auto target").await;

    // Configure the auto-add BEFORE polling.
    client
        .set_podcast_auto_playlists(podcast_id, vec![playlist_id], None)
        .await
        .expect("set auto-playlist");

    // Poll → ingest new episodes → auto-add them to the playlist.
    client.poll_now().await.expect("poll");

    let in_playlist = client
        .list_playlist_episodes(playlist_id, ep_page(0, 200))
        .await
        .expect("list playlist episodes")
        .data;
    assert!(
        !in_playlist.is_empty(),
        "new episodes were auto-added to the configured playlist"
    );

    // Every ingested episode of the podcast is in the playlist.
    let ingested = client
        .list_episodes(ep_page(0, 200))
        .await
        .expect("list episodes")
        .data
        .into_iter()
        .filter(|e| e.podcast_id == podcast_id)
        .count();
    assert_eq!(
        in_playlist.len(),
        ingested,
        "all ingested episodes landed in the playlist"
    );
}

/// The per-podcast `add_to_start` override: with the link set to insert at the
/// START, a poll's newly-ingested episodes land BEFORE the playlist's existing
/// entries (the default — exercised by `auto_playlist_rss_poll_adds_new_episodes`
/// — appends at the end).
#[tokio::test]
async fn auto_playlist_add_to_start_inserts_before_existing() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let feed = load_feed("hanselminutes.xml");
    let upstream = mock_feed(&feed).await;
    let podcast_id = app.seed_podcast("Auto Front", &upstream.uri()).await;
    let playlist_id = make_playlist(&app, "Front target").await;

    // A pre-existing playlist entry (a synthetic episode the feed doesn't carry):
    // auto-added episodes must land in front of it, not after.
    let existing = app.seed_episodes(podcast_id, 1).await;
    app.seed_playlist_episodes(playlist_id, &existing).await;

    // Link with the override; both set and get echo it back.
    let set = client
        .set_podcast_auto_playlists(podcast_id, vec![playlist_id], Some(true))
        .await
        .expect("set auto-playlist (front)");
    assert_eq!(set[0].add_to_start, Some(true), "set echoes the override");
    let got = client
        .get_podcast_auto_playlists(podcast_id)
        .await
        .expect("get auto-playlists");
    assert_eq!(got[0].add_to_start, Some(true), "override persisted");

    client.poll_now().await.expect("poll");

    // List in explicit position order (the queue's play order) — the harness's
    // bare `ep_page` defaults to id order, which would hide the insert position.
    let in_playlist = client
        .list_playlist_episodes(
            playlist_id,
            ep_params(
                0,
                2000,
                Some(("position", OrderDirection::Asc)),
                vec![],
                None,
            ),
        )
        .await
        .expect("list playlist episodes")
        .data;
    assert!(
        in_playlist.len() > 1,
        "auto-added episodes joined the pre-existing entry"
    );
    assert_eq!(
        in_playlist.last().map(|e| e.id),
        existing.first().copied(),
        "the pre-existing entry stays last — new episodes were inserted at the start"
    );
}
