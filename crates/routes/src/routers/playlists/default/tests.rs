use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

/// With a seeded default playlist, `/playlists/default` returns it.
#[tokio::test]
async fn returns_the_default_playlist() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    // Promote playlist_id_1 to default first (the fixture seeds none as default).
    let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();
    let promote_body = serde_json::json!({"data":{"is_default":true}});
    let promote = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/playlists/{playlist_id_1}"),
            &token,
            &promote_body,
        ))
        .await
        .unwrap();
    assert_eq!(promote.status(), StatusCode::OK);

    let response = router
        .oneshot(authed("GET", "/api/v1/playlists/default", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let pl = json.get("data").unwrap().get("playlist").unwrap();
    assert_eq!(
        pl.get("id").unwrap().as_i64().unwrap(),
        playlist_id_1.parse::<i64>().unwrap(),
        "default endpoint returns the promoted playlist; body: {json}"
    );
}

/// With no default seeded, `/playlists/default` returns `playlist: null` (200,
/// not a 404) — "no queue" is a state, not an error.
#[tokio::test]
async fn returns_null_when_no_default() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(authed("GET", "/api/v1/playlists/default", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert!(
        json.get("data").unwrap().get("playlist").unwrap().is_null(),
        "no default → playlist null; body: {json}"
    );
}
