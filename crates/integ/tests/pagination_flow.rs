//! Verify predictable pagination, including the size-10 default, through ApiClient's real serde_qs encoding. This
//! supports full worker pulls and lazy list/ID windows. Run the halogen-integ pagination_flow binary.

use halogen_integ::*;
use halogen_wire::{
    DefaultListParams, OrderDirection, Pagination, PlaybackListParams, PlaylistInclude,
    PodcastInclude,
};

#[tokio::test]
async fn episodes_list_reports_paginator_and_orders_by_published_at_desc() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let podcast_id = app
        .seed_podcast("Test Podcast", "https://feed.test/rss")
        .await;
    app.seed_episodes(podcast_id, 25).await;

    let result = client
        .list_episodes(ep_params(
            1,
            10,
            Some(("published_at", OrderDirection::Desc)),
            vec![],
            None,
        ))
        .await
        .expect("list");

    // Paginator is populated (it used to be null) and reports the totals.
    let p = result.paginator.expect("paginator populated");
    assert_eq!(p.page, 1, "echoes requested page");
    assert_eq!(p.size, 10, "echoes requested size");
    assert_eq!(p.total, 25, "total across all pages");
    assert_eq!(p.pages, 3, "ceil(25/10) = 3 pages");

    // The page itself is full and ordered newest-first by published_at.
    assert_eq!(result.data.len(), 10, "page 1 of size 10 is full");
    for w in result.data.windows(2) {
        assert!(
            w[0].published_at >= w[1].published_at,
            "published_at must be descending across the page"
        );
    }
}

#[tokio::test]
async fn episodes_paginate_past_the_default_page() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let podcast_id = app
        .seed_podcast("Test Podcast", "https://feed.test/rss")
        .await;
    app.seed_episodes(podcast_id, 25).await;

    // No pagination param → the server's default page size (10). This silent
    // default is exactly what capped the client before it learned to page.
    let default = client
        .list_episodes(DefaultListParams::default())
        .await
        .expect("default");
    assert_eq!(default.data.len(), 10, "no pagination → default size 10");

    // One big page → the whole library.
    assert_eq!(
        client
            .list_episodes(ep_page(0, 200))
            .await
            .expect("big")
            .data
            .len(),
        25,
        "size=200 → all 25"
    );

    // Walking pages of size 10 yields 10, 10, then the final 5.
    assert_eq!(
        client
            .list_episodes(ep_page(1, 10))
            .await
            .expect("p1")
            .data
            .len(),
        10,
        "page 1 size 10 → next 10"
    );
    assert_eq!(
        client
            .list_episodes(ep_page(2, 10))
            .await
            .expect("p2")
            .data
            .len(),
        5,
        "page 2 size 10 → final 5"
    );
}

#[tokio::test]
async fn podcasts_paginate_past_the_default_page() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    for i in 0..15 {
        app.seed_podcast(
            &format!("Podcast {i:02}"),
            &format!("https://feed.test/{i}"),
        )
        .await;
    }

    let default = client
        .list_podcasts(list_default::<PodcastInclude>())
        .await
        .expect("default");
    assert_eq!(default.data.len(), 10, "default size 10");
    assert_eq!(
        client
            .list_podcasts(list_all::<PodcastInclude>())
            .await
            .expect("all")
            .data
            .len(),
        15,
        "size 200 → all 15"
    );
}

#[tokio::test]
async fn playlists_paginate_past_the_default_page() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    for i in 0..15 {
        app.seed_playlist(&format!("Playlist {i:02}"), i == 0).await;
    }

    let default = client
        .list_playlists(list_default::<PlaylistInclude>())
        .await
        .expect("default");
    assert_eq!(default.data.len(), 10, "default size 10");
    assert_eq!(
        client
            .list_playlists(list_all::<PlaylistInclude>())
            .await
            .expect("all")
            .data
            .len(),
        15,
        "size 200 → all 15"
    );
}

#[tokio::test]
async fn playbacks_paginate_past_the_default_page() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let podcast_id = app
        .seed_podcast("Test Podcast", "https://feed.test/rss")
        .await;
    let episode_ids = app.seed_episodes(podcast_id, 15).await;
    app.seed_playbacks(admin.id, &episode_ids).await;

    let default = client
        .list_playbacks(PlaybackListParams::default())
        .await
        .expect("default");
    assert_eq!(default.data.len(), 10, "default size 10");
    assert_eq!(
        client
            .list_playbacks(PlaybackListParams {
                pagination: Some(Pagination { page: 0, size: 200 }),
                ..Default::default()
            })
            .await
            .expect("all")
            .data
            .len(),
        15,
        "size 200 → all 15"
    );
}
