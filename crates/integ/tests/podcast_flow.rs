//! Podcast journeys. `podcast_journey` walks the full CRUD lifecycle (create →
//! list → get → update → list episodes → delete → confirm gone);
//! `delete_podcast_cascades` proves a delete also removes the podcast's episodes
//! and their playlist memberships + playback history. All via the real
//! `ApiClient`, asserting the resulting data — not just status codes.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(podcast_flow)'`

use halogen_api::ApiClient;
use halogen_integ::*;
use halogen_wire::{
    DefaultListParams, EpisodeInclude, FilterParams, PlaybackStoreData, PlaylistInclude,
    PlaylistStoreData, PodcastInclude, PodcastStoreData, PodcastUpdateData,
};

/// List params for podcasts with a large page so the whole library comes back.
fn pc_page() -> DefaultListParams<PodcastInclude> {
    DefaultListParams {
        pagination: Some(halogen_wire::Pagination { page: 0, size: 200 }),
        ..Default::default()
    }
}

/// Episodes filtered to one podcast (the nested-route equivalent the UI uses).
fn eps_of(podcast_id: i32) -> DefaultListParams<EpisodeInclude> {
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
async fn podcast_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // 1) Library starts empty.
    let page = client.list_podcasts(pc_page()).await.expect("list");
    assert!(page.data.is_empty(), "no podcasts yet");

    // 2) Create a podcast; the returned row echoes what we sent.
    let created = client
        .create_podcast(PodcastStoreData {
            title: "Software Engineering Daily".into(),
            feed_url: "https://example.com/sed-feed.xml".into(),
            description: Some("A daily podcast about software.".into()),
            art_url: None,
            author: None,
            podcast_config_id: None,
        })
        .await
        .expect("create");
    assert_eq!(created.title, "Software Engineering Daily");
    assert_eq!(created.feed_url, "https://example.com/sed-feed.xml");
    let podcast_id = created.id;

    // 3) It's now the single listed podcast.
    let page = client.list_podcasts(pc_page()).await.expect("list");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, podcast_id);

    // 4) Fetchable by id with the same fields.
    let got = client.get_podcast(podcast_id).await.expect("get");
    assert_eq!(got.id, podcast_id);
    assert_eq!(got.title, "Software Engineering Daily");

    // 5) Update the title; the change persists (re-get confirms).
    let updated = client
        .update_podcast(
            podcast_id,
            PodcastUpdateData {
                title: Some("SED (renamed)".into()),
                ..Default::default()
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.title, "SED (renamed)");
    assert_eq!(
        client.get_podcast(podcast_id).await.expect("re-get").title,
        "SED (renamed)",
        "update persisted"
    );

    // 6) No episodes ingested for this podcast yet.
    let eps = client.list_episodes(eps_of(podcast_id)).await.expect("eps");
    assert!(eps.data.is_empty(), "no episodes ingested");

    // 7) Delete it, then GET-by-id is a 404.
    client.delete_podcast(podcast_id).await.expect("delete");
    let err = client.get_podcast(podcast_id).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "deleted podcast is gone");
}

/// Validation + 404 round-trips over the real wire: empty title / invalid
/// feed_url on create are 400, and get/update of a missing podcast id is 404.
#[tokio::test]
async fn podcast_validation_and_404() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Empty title → 400.
    let err = client
        .create_podcast(PodcastStoreData {
            title: "".into(),
            feed_url: "https://example.com/ok.xml".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 400, "empty title is a 400");

    // Invalid feed_url → 400.
    let err = client
        .create_podcast(PodcastStoreData {
            title: "Fine".into(),
            feed_url: "not-a-url".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 400, "invalid feed_url is a 400");

    // GET a never-existing podcast → 404.
    let err = client.get_podcast(999_999).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "missing podcast get is 404");

    // UPDATE a never-existing podcast → 404.
    let err = client
        .update_podcast(
            999_999,
            PodcastUpdateData {
                title: Some("ghost".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing podcast update is 404");
}

/// The podcast list + get report a server-computed `episode_count`, so the UI
/// can show per-podcast counts without holding every episode in memory.
#[tokio::test]
async fn podcast_reports_episode_count() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let podcast_id = app
        .seed_podcast("Counted", "https://feed.test/counted")
        .await;
    app.seed_episodes(podcast_id, 5).await;

    let page = client.list_podcasts(pc_page()).await.expect("list");
    let p = page
        .data
        .iter()
        .find(|p| p.id == podcast_id)
        .expect("podcast present");
    assert_eq!(p.episode_count, Some(5), "list reports episode_count");

    let got = client.get_podcast(podcast_id).await.expect("get");
    assert_eq!(got.episode_count, Some(5), "get reports episode_count");
}

/// Deleting a podcast cascades: its episodes, their playlist memberships, and
/// their playback history are all removed.
#[tokio::test]
async fn delete_podcast_cascades() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client: ApiClient = api(&app, &admin.token);
    // A queue (default playlist) must exist before non-default playlists can be
    // created (the first playlist must be the queue).
    app.seed_playlist("Queue", true).await;

    // Ingest a podcast + episodes from a mocked feed.
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;

    // Create a playlist and add the episode to it.
    let playlist = client
        .create_playlist(PlaylistStoreData {
            name: "Test".into(),
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

    // Record a playback for the episode.
    client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 120,
            completed: false,
        })
        .await
        .expect("create playback");

    // Sanity: the dependents exist before deletion.
    assert!(
        !client
            .list_episodes(eps_of(podcast_id))
            .await
            .expect("eps")
            .data
            .is_empty(),
        "episodes ingested"
    );
    assert_eq!(
        client
            .list_playbacks(Default::default())
            .await
            .expect("playbacks")
            .data
            .len(),
        1,
        "one playback"
    );
    assert_eq!(
        client
            .list_playlist_episodes(playlist.id, ep_page(0, 200))
            .await
            .expect("playlist eps")
            .data
            .len(),
        1,
        "episode is in the playlist"
    );

    // Delete the podcast.
    client.delete_podcast(podcast_id).await.expect("delete");

    // Everything that depended on the podcast's episodes is gone.
    assert!(
        client
            .list_episodes(eps_of(podcast_id))
            .await
            .expect("eps")
            .data
            .is_empty(),
        "episodes cascade-deleted"
    );
    assert!(
        client
            .list_playbacks(Default::default())
            .await
            .expect("playbacks")
            .data
            .is_empty(),
        "playback history cascade-deleted"
    );
    assert!(
        client
            .list_playlist_episodes(playlist.id, ep_page(0, 200))
            .await
            .expect("playlist eps")
            .data
            .is_empty(),
        "playlist membership cascade-deleted"
    );

    // The PlaylistInclude/`EpisodeIds` projection the sync worker uses also
    // reflects the now-empty playlist.
    let lists = client
        .list_playlists(DefaultListParams {
            pagination: Some(halogen_wire::Pagination { page: 0, size: 200 }),
            includes: Some(vec![PlaylistInclude::EpisodeIds]),
            ..Default::default()
        })
        .await
        .expect("list playlists");
    let pl = lists
        .data
        .iter()
        .find(|p| p.id == playlist.id)
        .expect("playlist present");
    assert!(
        pl.episode_ids.as_deref().unwrap_or(&[]).is_empty(),
        "EpisodeIds projection is empty after cascade"
    );
}

/// `filter.ids` narrows the podcast list to exactly the requested subscribed ids
/// (the client's batched pool-prime, one request instead of N), and — being ANDed
/// onto the subscription scope — can only narrow, never widen: a non-subscribed id
/// yields nothing rather than leaking another library.
#[tokio::test]
async fn list_podcasts_filter_ids_narrows() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let mk = |title: &str, feed: &str| PodcastStoreData {
        title: title.into(),
        feed_url: feed.into(),
        description: None,
        art_url: None,
        author: None,
        podcast_config_id: None,
    };
    let p1 = client
        .create_podcast(mk("One", "https://example.com/1.xml"))
        .await
        .expect("create")
        .id;
    let p2 = client
        .create_podcast(mk("Two", "https://example.com/2.xml"))
        .await
        .expect("create")
        .id;
    let p3 = client
        .create_podcast(mk("Three", "https://example.com/3.xml"))
        .await
        .expect("create")
        .id;

    // Unfiltered: all three subscribed podcasts.
    let all = client.list_podcasts(pc_page()).await.expect("list");
    assert_eq!(all.data.len(), 3);

    // filter.ids = [p1, p3] → exactly those two; p2 excluded.
    let ids_params = |ids: Vec<i32>| DefaultListParams::<PodcastInclude> {
        pagination: Some(halogen_wire::Pagination { page: 0, size: 200 }),
        filter: Some(FilterParams {
            ids: Some(ids),
            ..Default::default()
        }),
        ..Default::default()
    };
    let page = client
        .list_podcasts(ids_params(vec![p1, p3]))
        .await
        .expect("list ids");
    let mut got: Vec<i32> = page.data.iter().map(|p| p.id).collect();
    got.sort();
    let mut want = vec![p1, p3];
    want.sort();
    assert_eq!(got, want, "filter.ids returns exactly the requested ids");
    assert!(!page.data.iter().any(|p| p.id == p2), "p2 excluded");

    // A non-subscribed id (does not exist for this user) can't widen the result:
    // [p1, huge] returns only p1.
    let page = client
        .list_podcasts(ids_params(vec![p1, 2_000_000_000]))
        .await
        .expect("list ids w/ stranger");
    assert_eq!(
        page.data.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![p1],
        "non-subscribed id yields nothing (intersection, not union)"
    );
}
