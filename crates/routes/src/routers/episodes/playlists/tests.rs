use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, json_body, unauthed};
use axum::http::StatusCode;
use tower::ServiceExt;

/// episode_1 is seeded into playlist_1 → membership lists playlist_1.
#[tokio::test]
async fn test_episode_playlists_lists_members() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id_1 = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{}/playlists", episode_id_1),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let data = json.get("data").unwrap().as_array().unwrap();
    let ids: Vec<String> = data
        .iter()
        .map(|p| p.get("id").unwrap().to_string())
        .collect();
    assert_eq!(ids, vec![playlist_id_1.to_string()]);
}

/// episode_2 is in no playlist → empty membership.
#[tokio::test]
async fn test_episode_playlists_empty_when_no_membership() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id_2 = payload.get("episode_id_2").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{}/playlists", episode_id_2),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert!(json.get("data").unwrap().as_array().unwrap().is_empty());
}

/// Membership without a bearer token → 401.
#[tokio::test]
async fn test_episode_playlists_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let episode_id_1 = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(unauthed(
            "GET",
            format!("/api/v1/episodes/{}/playlists", episode_id_1),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
