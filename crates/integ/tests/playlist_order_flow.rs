//! End-to-end coverage for the playlist *ordering* / *membership* / *search* APIs
//! added for lazy playlist loading: the manual `position` column + `POST
//! /playlists/{id}/move`, `GET /episodes/{id}/playlists`, and the `filter[search]`
//! name filter on the playlist list. Driven through the real server + `ApiClient`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(playlist_order_flow)'`

use halogen_integ::*;
use halogen_wire::{
    DefaultListParams, EpisodeInclude, FilterParams, Order, OrderDirection, Pagination,
    PlaybackStatus, PlaylistInclude, PlaylistStoreData,
};

fn by_position() -> DefaultListParams<PlaylistInclude> {
    DefaultListParams {
        pagination: Some(Pagination { page: 0, size: 200 }),
        order: Some(Order {
            direction: OrderDirection::Asc,
            order_by: "position".into(),
        }),
        ..Default::default()
    }
}

/// The user's playlist ids in manual (`position`) order.
async fn ids_by_position(client: &halogen_api::ApiClient) -> Vec<i32> {
    client
        .list_playlists(by_position())
        .await
        .expect("list playlists")
        .data
        .iter()
        .map(|p| p.id)
        .collect()
}

/// New playlists are appended (`position = max + 1`); reordering one rewrites every
/// playlist's position 0..n, so the position-ordered list reflects the new order.
/// Out-of-range clamps to the last slot; an unchanged target is a no-op.
#[tokio::test]
async fn playlist_move_reorders_user_playlists() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // A queue (default) must exist before non-default creates; it lands at position 0.
    app.seed_playlist("Queue", true).await;

    // Created via the API → appended at max+1 (1, 2, 3).
    let a = client.create_playlist(named("A")).await.expect("a").id;
    let b = client.create_playlist(named("B")).await.expect("b").id;
    let c = client.create_playlist(named("C")).await.expect("c").id;

    let start = ids_by_position(&client).await;
    let queue = start[0];
    assert_eq!(
        start,
        vec![queue, a, b, c],
        "creation order is the initial manual order"
    );

    // Move C to index 1 → [queue, C, A, B].
    client.move_playlist(c, 1).await.expect("move c to 1");
    assert_eq!(ids_by_position(&client).await, vec![queue, c, a, b]);

    // Out-of-range target clamps to the last slot → move queue to 99 → [C, A, B, queue].
    client.move_playlist(queue, 99).await.expect("clamp");
    assert_eq!(ids_by_position(&client).await, vec![c, a, b, queue]);

    // Moving to the current index is a no-op.
    client.move_playlist(c, 0).await.expect("no-op");
    assert_eq!(ids_by_position(&client).await, vec![c, a, b, queue]);

    // Moving a playlist that doesn't exist → 404.
    let err = client.move_playlist(999_999, 0).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "moving a missing playlist is 404");
}

/// `GET /episodes/{id}/playlists` returns exactly the caller's playlists that
/// contain the episode (the picker's pre-selection), and an empty page otherwise.
#[tokio::test]
async fn episode_playlists_lists_membership() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    app.seed_playlist("Queue", true).await;
    let a = client.create_playlist(named("A")).await.expect("a").id;
    let b = client.create_playlist(named("B")).await.expect("b").id;
    let c = client.create_playlist(named("C")).await.expect("c").id;

    let podcast_id = app.seed_podcast("P", "https://feed.test/order").await;
    let e0 = app
        .seed_episode(
            podcast_id,
            "E0",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;
    let e1 = app
        .seed_episode(
            podcast_id,
            "E1",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;

    // e0 goes into A and C (not B); e1 into nothing.
    client.add_episode(a, e0, None).await.expect("add e0->a");
    client.add_episode(c, e0, None).await.expect("add e0->c");

    let params = DefaultListParams::<PlaylistInclude> {
        pagination: Some(Pagination { page: 0, size: 200 }),
        ..Default::default()
    };
    let mut members: Vec<i32> = client
        .list_episode_playlists(e0, params.clone())
        .await
        .expect("e0 membership")
        .data
        .iter()
        .map(|p| p.id)
        .collect();
    members.sort_unstable();
    let mut want = vec![a, c];
    want.sort_unstable();
    assert_eq!(members, want, "e0 is in A and C only");
    assert!(!members.contains(&b), "B is excluded");

    let empty = client
        .list_episode_playlists(e1, params)
        .await
        .expect("e1 membership")
        .data;
    assert!(empty.is_empty(), "e1 is in no playlist");
}

/// `add_episode`'s optional `position` controls where the episode lands: `None`
/// appends (default), `Some(0)` inserts at the front and renumbers the rest. The
/// position-ordered listing reflects it — this backs the "add to front of queue"
/// client setting.
#[tokio::test]
async fn add_episode_position_inserts_at_front() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let queue = app.seed_playlist("Queue", true).await;
    let podcast_id = app.seed_podcast("P", "https://feed.test/front").await;
    let mut eps = Vec::new();
    for t in ["E0", "E1", "E2"] {
        eps.push(
            app.seed_episode(
                podcast_id,
                t,
                "",
                chrono::Utc::now(),
                PlaybackStatus::Unplayed,
            )
            .await,
        );
    }
    let (e0, e1, e2) = (eps[0], eps[1], eps[2]);

    // Append e0, e1 → [e0, e1]; then insert e2 at the front → [e2, e0, e1].
    client
        .add_episode(queue, e0, None)
        .await
        .expect("append e0");
    client
        .add_episode(queue, e1, None)
        .await
        .expect("append e1");
    client
        .add_episode(queue, e2, Some(0))
        .await
        .expect("front e2");

    // Queue (position) order is opt-in via `order_by=position`; with no order the
    // list defaults to id order.
    let params = DefaultListParams::<EpisodeInclude> {
        pagination: Some(Pagination { page: 0, size: 200 }),
        order: Some(Order {
            order_by: "position".to_string(),
            direction: OrderDirection::Asc,
        }),
        ..Default::default()
    };
    let order: Vec<i32> = client
        .list_playlist_episodes(queue, params)
        .await
        .expect("list queue episodes")
        .data
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        order,
        vec![e2, e0, e1],
        "Some(0) inserts at the front; None appends"
    );
}

/// Position ordering must page over the *position*-sorted set, not slice by id and
/// reorder only the page. Regression for the bug where `order_by=position` paged by
/// episode id in SQL: with id order reversed from position order, page 0 returned
/// the lowest-id episodes (wrong) instead of the lowest-position ones.
#[tokio::test]
async fn position_paging_crosses_pages_by_position() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let queue = app.seed_playlist("Queue", true).await;
    let podcast_id = app.seed_podcast("P", "https://feed.test/paging").await;
    let mut eps = Vec::new();
    for t in ["E0", "E1", "E2", "E3"] {
        eps.push(
            app.seed_episode(
                podcast_id,
                t,
                "",
                chrono::Utc::now(),
                PlaybackStatus::Unplayed,
            )
            .await,
        );
    }
    // Insert each at the front → positions are the reverse of insertion (= id) order:
    // final position order is [e3, e2, e1, e0] while ids ascend e0 < e1 < e2 < e3.
    for &e in &eps {
        client.add_episode(queue, e, Some(0)).await.expect("front");
    }
    let (e0, e1, e2, e3) = (eps[0], eps[1], eps[2], eps[3]);

    let page = |p: i32| DefaultListParams::<EpisodeInclude> {
        pagination: Some(Pagination { page: p, size: 2 }),
        order: Some(Order {
            order_by: "position".to_string(),
            direction: OrderDirection::Asc,
        }),
        ..Default::default()
    };
    let ids = |resp: halogen_api::Page<Vec<halogen_wire::EpisodeData>>| {
        resp.data.iter().map(|e| e.id).collect::<Vec<_>>()
    };

    let p0 = client
        .list_playlist_episodes(queue, page(0))
        .await
        .expect("page 0");
    assert_eq!(
        p0.paginator.as_ref().unwrap().total,
        4,
        "total is the full set"
    );
    assert_eq!(
        ids(p0),
        vec![e3, e2],
        "page 0 is the two lowest-position episodes, not the two lowest-id"
    );

    let p1 = client
        .list_playlist_episodes(queue, page(1))
        .await
        .expect("page 1");
    assert_eq!(
        ids(p1),
        vec![e1, e0],
        "page 1 continues the position order across the page boundary"
    );
}

/// `filter[search]` narrows the playlist list by name (LIKE), so a lazy client can
/// page server-side matches rather than only filtering its cached pool.
#[tokio::test]
async fn playlist_search_filters_by_name() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    app.seed_playlist("Queue", true).await;
    let work_out = client
        .create_playlist(named("Work Out"))
        .await
        .expect("wo")
        .id;
    let work_trip = client
        .create_playlist(named("Work Trip"))
        .await
        .expect("wt")
        .id;
    let _chill = client
        .create_playlist(named("Chill"))
        .await
        .expect("chill")
        .id;

    let params = DefaultListParams::<PlaylistInclude> {
        pagination: Some(Pagination { page: 0, size: 200 }),
        filter: Some(FilterParams {
            search: Some("Work".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut got: Vec<i32> = client
        .list_playlists(params)
        .await
        .expect("search")
        .data
        .iter()
        .map(|p| p.id)
        .collect();
    got.sort_unstable();
    let mut want = vec![work_out, work_trip];
    want.sort_unstable();
    assert_eq!(got, want, "only the 'Work' playlists match");
}

fn named(name: &str) -> PlaylistStoreData {
    PlaylistStoreData {
        name: name.into(),
        description: None,
        is_default: None,
        ..Default::default()
    }
}
