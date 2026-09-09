//! Playback journey — save and resume a playback position. Steps build on each other and prove the
//! resume-position upsert (the unique-index path), the `episode_id` filter, and deletion. Driven through the
//! `ApiClient`, asserting the persisted values. Run with: `cargo nextest run -p halogen-integ -E
//! 'binary(playback_flow)'`

use halogen_integ::*;
use halogen_wire::{PlaybackListParams, PlaybackStoreData};

#[tokio::test]
async fn playback_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Arrange: ingest two episodes to play.
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    // A second distinct episode for the episode_id filter assertion.
    let other_id = app
        .seed_episode(
            podcast_id,
            "Another",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;

    // 1) No playbacks yet.
    let page = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("list");
    assert!(page.data.is_empty(), "no playbacks yet");

    // 2) Save a cursor. The owner is derived from the JWT, not the body.
    let saved = client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 120,
            completed: false,
        })
        .await
        .expect("upsert");
    assert_eq!(saved.cursor, 120);
    assert_eq!(saved.user_id, admin.id);
    assert!(!saved.completed);

    // 3) It's listed (exactly one row, with the saved cursor).
    let page = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("list");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].cursor, 120);

    // 4) Resuming saves a NEW cursor on the SAME row (upsert, not duplicate).
    let resumed = client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 300,
            completed: false,
        })
        .await
        .expect("upsert resume");
    assert_eq!(resumed.cursor, 300, "cursor advanced");
    let page = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("list");
    assert_eq!(
        page.data.len(),
        1,
        "upserts on (user, episode), no duplicate"
    );
    assert_eq!(page.data[0].cursor, 300, "persisted the advanced cursor");

    // 5) Marking played sets `completed` and still upserts (one row).
    let done = client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 0,
            completed: true,
        })
        .await
        .expect("upsert completed");
    assert!(done.completed, "marked completed");
    let page = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("list");
    assert_eq!(page.data.len(), 1, "still one row after marking played");

    // 6) A second episode's playback, then the `episode_id` filter returns only
    //  that one row.
    client
        .upsert_playback(PlaybackStoreData {
            episode_id: other_id,
            cursor: 42,
            completed: false,
        })
        .await
        .expect("upsert other");
    let filtered = client
        .list_playbacks(PlaybackListParams {
            episode_id: Some(other_id),
            ..Default::default()
        })
        .await
        .expect("filtered list");
    assert_eq!(filtered.data.len(), 1, "episode_id filter narrows to one");
    assert_eq!(filtered.data[0].episode_id, other_id);
    assert_eq!(filtered.data[0].cursor, 42);

    // 7) Delete that playback; it disappears, the other remains.
    let pb_id = filtered.data[0].id;
    client
        .delete_playback(pb_id)
        .await
        .expect("delete playback");
    let remaining = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("list");
    assert_eq!(remaining.data.len(), 1, "one playback left after delete");
    assert_eq!(remaining.data[0].episode_id, episode_id);
}

/// Playbacks are per-user: one user's stored playback is invisible to another.
/// A second user GETs the admin's playback id → 404, and their list is empty.
/// (No typed `get_playback` on the client, so the by-id GET uses raw HTTP.)
#[tokio::test]
async fn playbacks_are_isolated_per_user() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let admin_client = api(&app, &admin.token);
    let (_other_id, other_token) = app.seed_user("listener").await;

    // Arrange: an episode the admin records a playback against.
    let podcast_id = app.seed_podcast("Iso", "https://feed.test/iso").await;
    let episode_id = app
        .seed_episode(
            podcast_id,
            "Ep",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;

    // Admin stores a playback; capture its id.
    let saved = admin_client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 99,
            completed: false,
        })
        .await
        .expect("admin upsert");
    let admin_pb_id = saved.id;
    assert_eq!(saved.user_id, admin.id);

    // The second user's playback list is empty — they own none.
    let other_client = api(&app, &other_token);
    let other_list = other_client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("other list");
    assert!(
        other_list.data.is_empty(),
        "the second user sees none of the admin's playbacks"
    );

    // The second user GET-by-id of the admin's playback → 404 (owner-scoped).
    let http = reqwest::Client::new();
    let by_id = http
        .get(format!("{}/api/v1/playbacks/{admin_pb_id}", app.base_url))
        .bearer_auth(&other_token)
        .send()
        .await
        .expect("get by id");
    assert_eq!(
        by_id.status().as_u16(),
        404,
        "another user's playback id must 404 for a non-owner"
    );

    // A never-existing playback id is also a 404 for the owner.
    let missing = http
        .get(format!("{}/api/v1/playbacks/{}", app.base_url, i32::MAX))
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("get missing");
    assert_eq!(missing.status().as_u16(), 404, "missing playback id is 404");
}
