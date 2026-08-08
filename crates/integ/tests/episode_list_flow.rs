//! Episode list matrix — the lazy-loading / filtering / searching / ordering
//! contract the UI's paged episode list depends on, every case driven through
//! `ApiClient::list_episodes` (server impl in `handlers/episode/episode_list.rs`).
//! Each test asserts the exact returned set/sequence, not just a 200 — so a
//! `serde_qs` round-trip regression or a wrong filter surfaces immediately.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(episode_list_flow)'`

use chrono::{Duration, Utc};
use halogen_integ::*;
use halogen_wire::{DownloadStatus, EpisodeInclude, FilterParams, OrderDirection, PlaybackStatus};

fn ids_of(eps: &[halogen_wire::EpisodeData]) -> Vec<i32> {
    eps.iter().map(|e| e.id).collect()
}

/// `filter[search]` does a `LIKE %x%` over episode title, description, AND the
/// joined podcast title — ranked title > podcast > description — and is ASCII
/// case-insensitive. Non-matches are excluded.
#[tokio::test]
async fn search_matches_title_podcast_description_ranked() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let plain = app
        .seed_podcast("Cooking Show", "https://feed.test/c")
        .await;
    let rusty = app.seed_podcast("Rust Weekly", "https://feed.test/r").await;
    let now = Utc::now();

    let by_title = app
        .seed_episode(
            plain,
            "Rust intro",
            "nothing",
            now,
            PlaybackStatus::Unplayed,
        )
        .await;
    let by_podcast = app
        .seed_episode(rusty, "Cooking", "nothing", now, PlaybackStatus::Unplayed)
        .await;
    let by_desc = app
        .seed_episode(
            plain,
            "Cooking",
            "all about rust",
            now,
            PlaybackStatus::Unplayed,
        )
        .await;
    let _no_match = app
        .seed_episode(plain, "Cooking", "pasta", now, PlaybackStatus::Unplayed)
        .await;

    let search = |q: &str| {
        ep_params(
            0,
            200,
            None,
            vec![EpisodeInclude::Podcast],
            Some(FilterParams {
                search: Some(q.to_string()),
                ..Default::default()
            }),
        )
    };

    let got = client.list_episodes(search("rust")).await.expect("search");
    assert_eq!(
        ids_of(&got.data),
        vec![by_title, by_podcast, by_desc],
        "ranked title > podcast > description; non-match excluded"
    );

    // ASCII case-insensitive: an uppercase query returns the same set.
    let upper = client.list_episodes(search("RUST")).await.expect("search");
    assert_eq!(ids_of(&upper.data), vec![by_title, by_podcast, by_desc]);

    // A query that matches nothing returns an empty page.
    let none = client
        .list_episodes(search("zzz-nothing-matches"))
        .await
        .expect("search");
    assert!(none.data.is_empty(), "no matches → empty");
}

/// `filter[podcast_id]` returns only that podcast's episodes.
#[tokio::test]
async fn filter_by_podcast_id() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let a = app.seed_podcast("A", "https://feed.test/a").await;
    let b = app.seed_podcast("B", "https://feed.test/b").await;
    let a_eps = app.seed_episodes(a, 3).await;
    app.seed_episodes(b, 4).await;

    let got = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("id", OrderDirection::Asc)),
            vec![],
            Some(FilterParams {
                podcast_id: Some(a),
                ..Default::default()
            }),
        ))
        .await
        .expect("list");
    let mut want = a_eps.clone();
    want.sort_unstable();
    assert_eq!(ids_of(&got.data), want, "only podcast A's episodes");
}

/// `filter[download_status]` returns exactly the episodes in that state.
#[tokio::test]
async fn filter_by_download_status() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let ids = app.seed_episodes(p, 5).await;
    let downloaded = [ids[1], ids[3]];
    app.set_download_status(&downloaded, DownloadStatus::Downloaded)
        .await;

    let got = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("id", OrderDirection::Asc)),
            vec![],
            Some(FilterParams {
                download_status: Some("DOWNLOADED".into()),
                ..Default::default()
            }),
        ))
        .await
        .expect("list");
    let mut want = downloaded.to_vec();
    want.sort_unstable();
    assert_eq!(ids_of(&got.data), want, "only the Downloaded episodes");
}

/// `filter[playback_status]` returns exactly the episodes in that listen state.
#[tokio::test]
async fn filter_by_playback_status() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let now = Utc::now();
    let finished = app
        .seed_episode(p, "A", "", now, PlaybackStatus::Finished)
        .await;
    let _unplayed = app
        .seed_episode(p, "B", "", now, PlaybackStatus::Unplayed)
        .await;
    let _played = app
        .seed_episode(p, "C", "", now, PlaybackStatus::Played)
        .await;

    let got = client
        .list_episodes(ep_params(
            0,
            200,
            None,
            vec![],
            Some(FilterParams {
                playback_status: Some("FINISHED".into()),
                ..Default::default()
            }),
        ))
        .await
        .expect("list");
    assert_eq!(
        ids_of(&got.data),
        vec![finished],
        "only the Finished episode"
    );
}

/// `filter[published_after]` returns only episodes at/after the timestamp.
#[tokio::test]
async fn filter_by_published_after() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let base = Utc::now();
    let old = app
        .seed_episode(
            p,
            "old",
            "",
            base - Duration::days(10),
            PlaybackStatus::Unplayed,
        )
        .await;
    let mid = app
        .seed_episode(
            p,
            "mid",
            "",
            base - Duration::days(5),
            PlaybackStatus::Unplayed,
        )
        .await;
    let new = app
        .seed_episode(
            p,
            "new",
            "",
            base - Duration::days(1),
            PlaybackStatus::Unplayed,
        )
        .await;

    let got = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("published_at", OrderDirection::Asc)),
            vec![],
            Some(FilterParams {
                published_after: Some(base - Duration::days(5)),
                ..Default::default()
            }),
        ))
        .await
        .expect("list");
    assert_eq!(ids_of(&got.data), vec![mid, new], "mid (>=) and new only");
    assert!(!ids_of(&got.data).contains(&old), "old is excluded");
}

/// `filter[ids]` (the lazy id-list / `fetch_by_ids` path — the only *array*
/// filter, serialized `filter[ids][0]=…`) returns exactly the requested set.
/// This guards the nested-array `serde_qs` round-trip across the unified version.
#[tokio::test]
async fn filter_by_ids_returns_exactly_those() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let ids = app.seed_episodes(p, 10).await;
    let want = vec![ids[2], ids[5], ids[8]];

    let got = client
        .list_episodes(ep_params(
            0,
            200,
            Some(("id", OrderDirection::Asc)),
            vec![],
            Some(FilterParams {
                ids: Some(want.clone()),
                ..Default::default()
            }),
        ))
        .await
        .expect("list");
    let mut want_sorted = want.clone();
    want_sorted.sort_unstable();
    assert_eq!(ids_of(&got.data), want_sorted, "exactly the requested ids");
}

/// The order matrix: every `order_by` column the UI offers, both directions,
/// actually sorts the returned sequence that way. Titles, published_at, and
/// created_at are all distinct and deliberately *not* correlated so a wrong
/// column would reorder the result.
#[tokio::test]
async fn order_by_every_column_both_directions() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let base = Utc::now();
    // Titles alphabetical order (Alpha<Bravo<Charlie) differs from published order
    // (Charlie newest), so order_by=title vs published_at give different sequences.
    let charlie = app
        .seed_episode(p, "Charlie", "", base, PlaybackStatus::Unplayed)
        .await;
    let alpha = app
        .seed_episode(
            p,
            "Alpha",
            "",
            base - Duration::minutes(1),
            PlaybackStatus::Unplayed,
        )
        .await;
    let bravo = app
        .seed_episode(
            p,
            "Bravo",
            "",
            base - Duration::minutes(2),
            PlaybackStatus::Unplayed,
        )
        .await;

    let order = |by: &str, dir: OrderDirection| ep_params(0, 200, Some((by, dir)), vec![], None);

    // published_at: created_at mirrors it (seed_episode sets both), so both columns
    // sort the same way.
    for col in ["published_at", "created_at", "updated_at"] {
        let desc = client
            .list_episodes(order(col, OrderDirection::Desc))
            .await
            .expect("desc");
        assert_eq!(
            ids_of(&desc.data),
            vec![charlie, alpha, bravo],
            "{col} desc → newest first"
        );
        let asc = client
            .list_episodes(order(col, OrderDirection::Asc))
            .await
            .expect("asc");
        assert_eq!(
            ids_of(&asc.data),
            vec![bravo, alpha, charlie],
            "{col} asc → oldest first"
        );
    }

    // title: alphabetical, independent of the time columns.
    let title_asc = client
        .list_episodes(order("title", OrderDirection::Asc))
        .await
        .expect("title asc");
    assert_eq!(
        ids_of(&title_asc.data),
        vec![alpha, bravo, charlie],
        "title asc → Alpha, Bravo, Charlie"
    );
    let title_desc = client
        .list_episodes(order("title", OrderDirection::Desc))
        .await
        .expect("title desc");
    assert_eq!(
        ids_of(&title_desc.data),
        vec![charlie, bravo, alpha],
        "title desc → Charlie, Bravo, Alpha"
    );
}

/// List-parameter rejection + characterization matrix, driven over raw HTTP so
/// we control the exact query string (the typed client can't build invalid
/// params). `params.validate()` in `routers/episodes/list.rs` validates
/// pagination range and the `order_by` regex; it does NOT validate filter status
/// values or unknown-but-well-formed order columns (those are handled in the
/// query layer), so those are characterized as their current behavior.
#[tokio::test]
async fn list_param_rejections_and_characterizations() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let http = reqwest::Client::new();

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    let ids = app.seed_episodes(p, 3).await;

    let get = |query: &str| {
        let url = format!("{}/api/v1/episodes?{query}", app.base_url);
        let token = admin.token.clone();
        let http = http.clone();
        async move {
            http.get(url)
                .bearer_auth(&token)
                .send()
                .await
                .expect("list request")
        }
    };

    // Zero page size → validation 400 (Pagination.size min = 1).
    assert_eq!(
        get("pagination[page]=0&pagination[size]=0")
            .await
            .status()
            .as_u16(),
        400,
        "size=0 is a 400"
    );
    // Negative page → validation 400 (Pagination.page min = 0).
    assert_eq!(
        get("pagination[page]=-1&pagination[size]=10")
            .await
            .status()
            .as_u16(),
        400,
        "page=-1 is a 400"
    );

    // order_by with a disallowed character (space) → fails the ALPHA_DASH regex
    // in `Order::validate` → 400.
    assert_eq!(
        get("order[order_by]=pub lished&order[direction]=Asc")
            .await
            .status()
            .as_u16(),
        400,
        "order_by with a space is a 400 (regex)"
    );

    // CHARACTERIZATION: an unknown-but-well-formed order column passes validation
    // and the handler falls back to ordering by `id` → 200 (not a rejection).
    let resp = get("order[order_by]=bogus_column&order[direction]=Asc").await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "unknown order column falls back to id (200), not rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    let got: Vec<i64> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    let mut want: Vec<i64> = ids.iter().map(|&i| i as i64).collect();
    want.sort_unstable();
    assert_eq!(got, want, "fallback ordering is by id ascending");

    // CHARACTERIZATION: a garbage `filter[playback_status]` value is not
    // rejected — it filters by equality and simply matches nothing → 200 empty.
    let resp = get("filter[playback_status]=NOPE").await;
    assert_eq!(resp.status().as_u16(), 200, "garbage playback_status → 200");
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "garbage playback_status matches nothing"
    );

    // Same for a garbage `filter[download_status]` value.
    let resp = get("filter[download_status]=NOPE").await;
    assert_eq!(resp.status().as_u16(), 200, "garbage download_status → 200");
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "garbage download_status matches nothing"
    );
}

/// Drive the nested `GET /podcasts/{id}/episodes` route directly (every other
/// test reaches episodes via `filter[podcast_id]`). It returns that podcast's
/// episodes, and a nonexistent podcast id is characterized as 200-empty (the
/// handler injects the id into the filter with no existence check).
#[tokio::test]
async fn nested_podcast_episodes_route() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let http = reqwest::Client::new();

    let p = app.seed_podcast("Nested", "https://feed.test/n").await;
    let ids = app.seed_episodes(p, 4).await;

    // The nested route returns this podcast's episodes.
    let resp = http
        .get(format!(
            "{}/api/v1/podcasts/{p}/episodes?pagination[page]=0&pagination[size]=200",
            app.base_url
        ))
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("nested list");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        ids.len(),
        "nested route returns the podcast's episodes"
    );

    // CHARACTERIZATION: a nonexistent podcast id → 200 with an empty list (no
    // existence check on the path id).
    let resp = http
        .get(format!("{}/api/v1/podcasts/999999/episodes", app.base_url))
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("nested list missing");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "nonexistent podcast → 200 (not 404)"
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "nonexistent podcast → empty episode list"
    );
}

/// `includes[Podcast]` eager-loads the parent podcast onto each episode.
#[tokio::test]
async fn include_podcast_populates_parent() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("My Show", "https://feed.test/m").await;
    app.seed_episodes(p, 2).await;

    // Without the include, the parent is absent.
    let bare = client.list_episodes(ep_page(0, 200)).await.expect("bare");
    assert!(
        bare.data.iter().all(|e| e.podcast.is_none()),
        "no include → podcast not loaded"
    );

    // With it, every episode carries the correct parent.
    let enriched = client
        .list_episodes(ep_params(0, 200, None, vec![EpisodeInclude::Podcast], None))
        .await
        .expect("enriched");
    assert!(!enriched.data.is_empty());
    for e in &enriched.data {
        let parent = e.podcast.as_ref().expect("podcast populated by include");
        assert_eq!(parent.title, "My Show");
    }
}

/// `includes[Chapters]` eager-loads each episode's ordered chapter markers, on
/// both the list and the single-episode GET. Without the include the field stays
/// `None`; an episode with no chapters comes back as an empty vec; and a normal
/// listing of episodes-without-chapters still succeeds.
#[tokio::test]
async fn include_chapters_populates_ordered_markers() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("Chaptered", "https://feed.test/ch").await;
    let now = Utc::now();
    let with = app
        .seed_episode(p, "Has chapters", "", now, PlaybackStatus::Unplayed)
        .await;
    let without = app
        .seed_episode(p, "No chapters", "", now, PlaybackStatus::Unplayed)
        .await;
    // Seeded out of order on purpose — the handler must return them by start time.
    app.seed_chapters(with, &[("Middle", 60), ("Intro", 0), ("Outro", 600)])
        .await;

    // Without the include, chapters are never loaded (bare list still works for
    // episodes that have none — no panic, field is None).
    let bare = client.list_episodes(ep_page(0, 200)).await.expect("bare");
    assert!(
        bare.data.iter().all(|e| e.chapters.is_none()),
        "no include → chapters not loaded"
    );

    // With the include on the LIST: the chaptered episode is start-ordered, the
    // other comes back as an explicit empty vec (Some, not None).
    let enriched = client
        .list_episodes(ep_params(
            0,
            200,
            None,
            vec![EpisodeInclude::Chapters],
            None,
        ))
        .await
        .expect("enriched");
    let with_ep = enriched
        .data
        .iter()
        .find(|e| e.id == with)
        .expect("chaptered episode present");
    let titles: Vec<&str> = with_ep
        .chapters
        .as_ref()
        .expect("chapters populated")
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Intro", "Middle", "Outro"], "ordered by start");
    let starts: Vec<i32> = with_ep
        .chapters
        .as_ref()
        .unwrap()
        .iter()
        .map(|c| c.starts_at_secs)
        .collect();
    assert_eq!(starts, vec![0, 60, 600]);
    let without_ep = enriched
        .data
        .iter()
        .find(|e| e.id == without)
        .expect("chapterless episode present");
    assert_eq!(
        without_ep.chapters.as_deref(),
        Some(&[][..]),
        "no chapters → empty vec when include requested"
    );

    // Same contract on the single-episode GET (exercises `chapters_for`).
    let single = client
        .get_episode(with, &[EpisodeInclude::Chapters])
        .await
        .expect("get with chapters");
    assert_eq!(
        single.chapters.as_ref().expect("chapters populated").len(),
        3
    );
    let bare_single = client.get_episode(with, &[]).await.expect("get bare");
    assert!(
        bare_single.chapters.is_none(),
        "no include → chapters None on single GET"
    );
}

/// Lazy loading: paging size 30 with a populated paginator drives the UI's
/// `has_more = page + 1 < paginator.pages` cursor, and the last page is partial.
#[tokio::test]
async fn lazy_paging_has_more_and_partial_last_page() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let p = app.seed_podcast("P", "https://feed.test/p").await;
    app.seed_episodes(p, 65).await; // 3 pages of 30 → 30, 30, 5

    let page0 = client.list_episodes(ep_page(0, 30)).await.expect("p0");
    let pag = page0.paginator.expect("paginator");
    assert_eq!(pag.total, 65);
    assert_eq!(pag.pages, 3, "ceil(65/30) = 3");
    assert_eq!(page0.data.len(), 30, "first page full");
    assert!(0 + 1 < pag.pages, "page 0 has_more");

    let page1 = client.list_episodes(ep_page(1, 30)).await.expect("p1");
    assert_eq!(page1.data.len(), 30, "second page full");
    assert!(1 + 1 < pag.pages, "page 1 has_more");

    let page2 = client.list_episodes(ep_page(2, 30)).await.expect("p2");
    assert_eq!(page2.data.len(), 5, "last page partial");
    assert!(!(2 + 1 < pag.pages), "page 2 is the last — no more");

    // The three pages partition the library with no overlap.
    let mut all = ids_of(&page0.data);
    all.extend(ids_of(&page1.data));
    all.extend(ids_of(&page2.data));
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 65, "pages cover every episode exactly once");
}

/// Filters, ordering, and pagination compose: a filtered + ordered query still
/// pages correctly.
#[tokio::test]
async fn filter_order_and_pagination_compose() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let target = app.seed_podcast("Target", "https://feed.test/t").await;
    let other = app.seed_podcast("Other", "https://feed.test/o").await;
    let target_eps = app.seed_episodes(target, 15).await; // newest-first by index
    app.seed_episodes(other, 10).await; // noise that must be excluded

    // Filter to `target`, order by published_at desc, page size 10.
    let params = |page: i32| {
        ep_params(
            page,
            10,
            Some(("published_at", OrderDirection::Desc)),
            vec![],
            Some(FilterParams {
                podcast_id: Some(target),
                ..Default::default()
            }),
        )
    };
    let p0 = client.list_episodes(params(0)).await.expect("p0");
    assert_eq!(p0.paginator.expect("pag").total, 15, "filtered total = 15");
    assert_eq!(p0.data.len(), 10, "first filtered page full");
    let p1 = client.list_episodes(params(1)).await.expect("p1");
    assert_eq!(p1.data.len(), 5, "second filtered page partial");

    // All returned ids belong to the target podcast; none from `other`.
    let returned: std::collections::HashSet<i32> = ids_of(&p0.data)
        .into_iter()
        .chain(ids_of(&p1.data))
        .collect();
    let want: std::collections::HashSet<i32> = target_eps.into_iter().collect();
    assert_eq!(
        returned, want,
        "exactly the target podcast's episodes, paged"
    );
}
