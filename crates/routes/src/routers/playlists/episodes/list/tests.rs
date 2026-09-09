use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, json_body, unauthed};
use axum::http::StatusCode;
use tower::ServiceExt;

/// No bearer token → the auth middleware rejects with 401 before the handler runs.
#[tokio::test]
async fn test_list_playlist_episodes_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let playlist_id = payload.get("playlist_id_2").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(unauthed(
            "GET",
            format!("/api/v1/playlists/{}/episodes", playlist_id),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// An existing playlist with no episode associations → 200 + empty array.
/// `playlist_id_2` is seeded with no episode-playlist rows.
#[tokio::test]
async fn test_list_playlist_episodes_empty() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id = payload.get("playlist_id_2").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/playlists/{}/episodes", playlist_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let episodes = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(episodes.len(), 0);
}

/// After POSTing an episode into an empty playlist, listing returns it.
#[tokio::test]
async fn test_list_playlist_episodes_after_add() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    // playlist_id_2 starts empty; episode_id_2 is not associated with it.
    let playlist_id: i32 = payload
        .get("playlist_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let episode_id: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(user_id);

    // Add the episode to the playlist.
    let store_body = serde_json::json!({
        "data": { "playlist_id": playlist_id, "episode_id": episode_id }
    });
    let add_response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &token,
            &store_body,
        ))
        .await
        .unwrap();
    assert_eq!(add_response.status(), StatusCode::OK);

    // Now list — the episode should be present.
    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/playlists/{}/episodes", playlist_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let episodes = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(episodes.len(), 1);
    assert_eq!(
        episodes[0].get("id").unwrap().as_i64().unwrap() as i32,
        episode_id
    );
}

/// A param-less list comes back in PIVOT POSITION order, not id order. Regression:
/// `params.order.unwrap_or_default()` used to fill `order_by = "id"`, dead-coding the position default — so a
/// front-add (position 0) showed up at the END for clients that fetch without an explicit order (the iOS
/// queue). Also covers position Desc, which was silently returned ascending.
#[tokio::test]
async fn test_list_playlist_episodes_defaults_to_position_order() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id: i32 = payload
        .get("playlist_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    // id order: episode_id_2 < episode_id_1 — so appending e2 then
    // front-adding e1 makes position order [e1, e2] the REVERSE of id
    // order, distinguishing the two.
    let e1: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let e2: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(user_id);

    for (ep, body) in [
        (e2, serde_json::json!({ "data": {} })),
        (e1, serde_json::json!({ "data": { "position": 0 } })),
    ] {
        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                format!("/api/v1/playlists/{}/episodes/{}", playlist_id, ep),
                &token,
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let listed_ids = |json: &serde_json::Value| -> Vec<i32> {
        json.get("data")
            .and_then(|d| d.as_array())
            .expect("data is array")
            .iter()
            .map(|e| e.get("id").unwrap().as_i64().unwrap() as i32)
            .collect()
    };

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/playlists/{}/episodes", playlist_id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(listed_ids(&json), vec![e1, e2], "param-less = position asc");

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!(
                "/api/v1/playlists/{}/episodes?order%5Border_by%5D=position&order%5Bdirection%5D=Desc",
                playlist_id
            ),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(listed_ids(&json), vec![e2, e1], "position desc reverses");
}

/// Non-integer path id → the `Id` extractor rejects with 400.
#[tokio::test]
async fn test_list_playlist_episodes_non_integer_id_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/playlists/not-an-int/episodes",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
