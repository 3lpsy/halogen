use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, episode_model, json, playlist_model, user_model};
use axum::http::StatusCode;
use halogen_orm::episode_playlist::{
    ActiveModel as EpPivot, Column as PivotColumn, Entity as PivotEntity,
};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder,
};
use tower::ServiceExt;

/// Seed an owned playlist with three episodes whose title/published/duration sorts differ. Return playlist ID and
/// [a,b,c] in insertion order; pivot timestamps increase a < b < c.
async fn seed_reorder_playlist(dbc: &DatabaseConnection, user_id: i32) -> (i32, [i32; 3]) {
    let podcast_id = i32::MAX - 2; // created by setup_test_db
    let playlist_id = i32::MAX - 30;
    let mut pl = playlist_model(playlist_id, user_id);
    pl.name = Set("Reorder Playlist".to_string());
    pl.position = Set(2);
    pl.insert(dbc).await.expect("insert reorder playlist");

    let specs = [
        (i32::MAX - 31, "Charlie", 1000, 300, 1000),
        (i32::MAX - 32, "alpha", 3000, 100, 2000),
        (i32::MAX - 33, "Bravo", 2000, 200, 3000),
    ];
    let mut ids = [0i32; 3];
    for (pos, (eid, title, pub_ts, dur, added_ts)) in specs.into_iter().enumerate() {
        let mut ep = episode_model(eid, podcast_id);
        ep.title = Set(title.to_string());
        ep.content_url = Set(format!("https://example.com/{eid}.mp3"));
        ep.published_at = Set(Some(ts(pub_ts)));
        ep.duration_secs = Set(Some(dur));
        ep.insert(dbc).await.expect("insert reorder episode");

        EpPivot {
            episode_id: Set(eid),
            playlist_id: Set(playlist_id),
            position: Set(pos as i32),
            created_at: Set(ts(added_ts)),
            updated_at: Set(ts(added_ts)),
        }
        .insert(dbc)
        .await
        .expect("insert reorder pivot");
        ids[pos] = eid;
    }
    (playlist_id, ids)
}

fn ts(secs: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

/// The playlist's episode ids in current `position` order, read straight from
/// the pivot table — independent of any read endpoint's defaults.
async fn position_order(dbc: &DatabaseConnection, playlist_id: i32) -> Vec<i32> {
    PivotEntity::find()
        .filter(PivotColumn::PlaylistId.eq(playlist_id))
        .order_by(PivotColumn::Position, Order::Asc)
        .all(dbc)
        .await
        .expect("load pivot order")
        .into_iter()
        .map(|r| r.episode_id)
        .collect()
}

async fn reorder(
    router: &axum::Router,
    token: &str,
    playlist_id: i32,
    field: &str,
    direction: &str,
) -> StatusCode {
    let body = serde_json::json!({"data": {"field": field, "direction": direction}});
    let resp = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{playlist_id}/reorder-by"),
            token,
            &body,
        ))
        .await
        .unwrap();
    resp.status()
}

#[tokio::test]
async fn reorder_by_title_asc() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, [a, b, c]) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(&user_id.to_string());

    assert_eq!(
        reorder(&router, &token, playlist_id, "title", "Asc").await,
        StatusCode::OK
    );
    // alpha (b), Bravo (c), Charlie (a) — case-insensitive.
    assert_eq!(position_order(&dbc, playlist_id).await, vec![b, c, a]);
}

#[tokio::test]
async fn reorder_by_published_asc() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, [a, b, c]) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(&user_id.to_string());

    assert_eq!(
        reorder(&router, &token, playlist_id, "published", "Asc").await,
        StatusCode::OK
    );
    // 1000 (a), 2000 (c), 3000 (b).
    assert_eq!(position_order(&dbc, playlist_id).await, vec![a, c, b]);
}

#[tokio::test]
async fn reorder_by_duration_desc() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, [a, b, c]) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(&user_id.to_string());

    assert_eq!(
        reorder(&router, &token, playlist_id, "duration", "Desc").await,
        StatusCode::OK
    );
    // 300 (a), 200 (c), 100 (b).
    assert_eq!(position_order(&dbc, playlist_id).await, vec![a, c, b]);
}

#[tokio::test]
async fn reorder_by_added_desc() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, [a, b, c]) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(&user_id.to_string());

    assert_eq!(
        reorder(&router, &token, playlist_id, "added", "Desc").await,
        StatusCode::OK
    );
    // Most-recently added first: c, b, a.
    assert_eq!(position_order(&dbc, playlist_id).await, vec![c, b, a]);
}

#[tokio::test]
async fn reorder_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, _) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({"data": {"field": "title", "direction": "Asc"}});
    let resp = router
        .clone()
        .oneshot(json(
            "POST",
            format!("/api/v1/playlists/{playlist_id}/reorder-by"),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reorder_non_owner_forbidden() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, _) = seed_reorder_playlist(&dbc, user_id).await;
    // A different, real, non-admin user may not reorder someone else's playlist.
    let other_id = i32::MAX - 40;
    user_model(other_id, "other_user", "testother123", false)
        .insert(&dbc)
        .await
        .expect("insert other user");
    let router = build_test_router(dbc);
    let other_token = generate_jwt_token(&other_id.to_string());

    assert_eq!(
        reorder(&router, &other_token, playlist_id, "title", "Asc").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn reorder_rejects_unknown_field() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let (playlist_id, _) = seed_reorder_playlist(&dbc, user_id).await;
    let router = build_test_router(dbc);
    let token = generate_jwt_token(&user_id.to_string());

    let status = reorder(&router, &token, playlist_id, "nonsense", "Asc").await;
    assert_ne!(
        status,
        StatusCode::OK,
        "unknown sort field must be rejected"
    );
}
