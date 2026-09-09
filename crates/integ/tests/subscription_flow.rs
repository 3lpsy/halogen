//! Subscription journey — subscribe to a (mocked) feed and watch episodes flow through the real ingestion
//! pipeline, each step building on the last. Driven through the `ApiClient`, asserting the ingested data. Run
//! with: `cargo nextest run -p halogen-integ -E 'binary(subscription_flow)'`

use halogen_integ::*;
use halogen_wire::{EpisodeInclude, FilterParams, PlaybackStoreData, PlaylistStoreData};

fn eps_of(podcast_id: i32) -> halogen_wire::DefaultListParams<EpisodeInclude> {
    ep_params(
        0,
        200,
        None,
        vec![],
        Some(FilterParams {
            podcast_id: Some(podcast_id),
            ..Default::default()
        }),
    )
}

#[tokio::test]
async fn subscription_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // 1) Library starts empty.
    let page = client
        .list_podcasts(Default::default())
        .await
        .expect("list");
    assert!(page.data.is_empty(), "no podcasts yet");

    // 2) Subscribe to a podcast whose feed is served by wiremock.
    let feed = load_feed("sed_podcast.xml");
    let upstream = mock_feed(&feed).await;
    let podcast_id = app
        .seed_podcast("Software Engineering Daily", &upstream.uri())
        .await;

    // 3) Poll over real HTTP — the server fetches + parses the mocked feed.
    let op = client.poll_now().await.expect("poll");
    assert!(!op.message.is_empty(), "poll acknowledged with a message");

    // 4) The podcast is now listed.
    let page = client
        .list_podcasts(Default::default())
        .await
        .expect("list");
    assert_eq!(page.data.len(), 1, "podcast subscribed");

    // 5) Episodes were ingested, with real titles + content urls.
    let eps = client.list_episodes(eps_of(podcast_id)).await.expect("eps");
    let ingested = eps.data.len();
    assert!(ingested > 0, "episodes ingested from the feed");
    for ep in &eps.data {
        assert!(!ep.title.is_empty(), "ingested episode has a title");
        assert!(
            !ep.content_url.is_empty(),
            "ingested episode has a content url"
        );
    }

    // 6) A single episode is fetchable by id and matches its list row.
    let first = &eps.data[0];
    let one = client
        .get_episode(first.id, &[])
        .await
        .expect("get episode");
    assert_eq!(one.id, first.id);
    assert_eq!(one.title, first.title);

    // 7) Re-polling the same feed is idempotent — no duplicate episodes.
    client.poll_now().await.expect("re-poll");
    let after = client.list_episodes(eps_of(podcast_id)).await.expect("eps");
    assert_eq!(
        after.data.len(),
        ingested,
        "re-poll must not duplicate episodes"
    );
}

/// `DELETE /episodes/{id}` (the resource delete, NOT `/episodes/{id}/download`):
/// removing one episode 404s its subsequent GET and cascades to its playlist
/// membership and playback. The typed `ApiClient` doesn't expose this route, so
/// the delete goes over raw HTTP; the rest is verified through the client.
#[tokio::test]
async fn delete_episode_cascades_membership_and_playback() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let http = reqwest::Client::new();
    // A queue (default playlist) must exist before non-default creates.
    app.seed_playlist("Queue", true).await;

    // Ingest a podcast + episodes.
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let before = client.list_episodes(eps_of(podcast_id)).await.expect("eps");
    let before_count = before.data.len();
    assert!(before_count >= 2, "need >= 2 episodes to delete one");

    // Add the episode to a playlist and record a playback against it.
    let playlist = client
        .create_playlist(PlaylistStoreData {
            name: "Has Member".into(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("create playlist");
    client
        .add_episode(playlist.id, episode_id, None)
        .await
        .expect("add to playlist");
    client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 60,
            completed: false,
        })
        .await
        .expect("create playback");

    // Sanity: dependents exist.
    assert_eq!(
        client
            .list_playlist_episodes(playlist.id, ep_page(0, 200))
            .await
            .expect("playlist eps")
            .data
            .len(),
        1,
        "episode is a playlist member before delete"
    );
    assert_eq!(
        client
            .list_playbacks(Default::default())
            .await
            .expect("playbacks")
            .data
            .len(),
        1,
        "one playback before delete"
    );

    // Delete the episode (resource route).
    let resp = http
        .delete(format!("{}/api/v1/episodes/{episode_id}", app.base_url))
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("delete episode");
    assert!(
        resp.status().is_success(),
        "episode delete should 2xx, got {}",
        resp.status()
    );

    // Subsequent GET-by-id → 404.
    let err = client.get_episode(episode_id, &[]).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "deleted episode is gone");

    // The library shrank by exactly one.
    let after = client.list_episodes(eps_of(podcast_id)).await.expect("eps");
    assert_eq!(
        after.data.len(),
        before_count - 1,
        "exactly the one episode was removed"
    );

    // Cascade: playlist membership and playback for that episode are gone.
    assert!(
        client
            .list_playlist_episodes(playlist.id, ep_page(0, 200))
            .await
            .expect("playlist eps")
            .data
            .is_empty(),
        "playlist membership cascade-deleted"
    );
    assert!(
        client
            .list_playbacks(Default::default())
            .await
            .expect("playbacks")
            .data
            .is_empty(),
        "playback cascade-deleted"
    );
}

/// `POST /episodes` direct create (raw HTTP — the typed client has no
/// create-episode), then a validation-400 case (empty title and a bad
/// content_url).
#[tokio::test]
async fn create_episode_direct_and_validation_errors() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let http = reqwest::Client::new();

    let podcast_id = app.seed_podcast("Direct", "https://feed.test/direct").await;
    let url = format!("{}/api/v1/episodes", app.base_url);

    // Valid create → 200 and the episode is then fetchable by the listed id.
    let resp = http
        .post(&url)
        .bearer_auth(&admin.token)
        .json(&serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "Hand-made Episode",
                "description": "A hand-made episode body.",
                "content_url": "https://example.test/handmade.mp3"
            }
        }))
        .send()
        .await
        .expect("create episode");
    assert_eq!(resp.status().as_u16(), 200, "direct create succeeds");
    let body: serde_json::Value = resp.json().await.expect("json");
    let new_id = body["data"]["id"].as_i64().expect("new episode id") as i32;
    let got = client.get_episode(new_id, &[]).await.expect("get created");
    assert_eq!(got.title, "Hand-made Episode");

    // Empty title → 400.
    let resp = http
        .post(&url)
        .bearer_auth(&admin.token)
        .json(&serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "",
                "description": "ok",
                "content_url": "https://example.test/ok.mp3"
            }
        }))
        .send()
        .await
        .expect("create empty title");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "empty title is a validation 400"
    );

    // Bad content_url → 400.
    let resp = http
        .post(&url)
        .bearer_auth(&admin.token)
        .json(&serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "Fine Title",
                "description": "fine",
                "content_url": "not-a-valid-url"
            }
        }))
        .send()
        .await
        .expect("create bad url");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "invalid content_url is a validation 400"
    );

    // GET a never-existing episode → 404.
    let err = client.get_episode(999_999, &[]).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "missing episode get is 404");

    // UPDATE a never-existing episode → 404.
    let err = client
        .update_episode(
            999_999,
            halogen_wire::EpisodeUpdateData {
                title: Some("ghost".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing episode update is 404");
}
