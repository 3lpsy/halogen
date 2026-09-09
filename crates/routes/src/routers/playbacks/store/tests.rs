use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_store_playback_creates_playback() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id: i32 = payload
        .get("user_id")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(&user_id.to_string());

    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let store_data = serde_json::json!({
        "data": { "episode_id": episode_id, "cursor": 100, "completed": false },
        "params": null,
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token,
            &store_data,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let playback_id = json
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|id| id.as_i64())
        .expect("playback id");
    assert!(playback_id > 0, "playback should have an id");
    // The owner is taken from the JWT, never the body.
    assert_eq!(json["data"]["user_id"].as_i64(), Some(user_id as i64));
}

#[tokio::test]
async fn test_store_playback_advances_episode_playback_status() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let post = |cursor: i64, completed: bool| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let body = serde_json::json!({
                "data": { "episode_id": episode_id, "cursor": cursor, "completed": completed },
                "params": null,
            });
            let resp = router
                .oneshot(authed_json("POST", "/api/v1/playbacks", &token, &body))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    };
    let status = || {
        let router = router.clone();
        let token = token.clone();
        async move {
            let resp = router
                .oneshot(authed(
                    "GET",
                    format!("/api/v1/episodes/{episode_id}"),
                    &token,
                ))
                .await
                .unwrap();
            let json = json_body(resp).await;
            json["data"]["playback_status"]
                .as_str()
                .expect("playback_status present")
                .to_string()
        }
    };

    // Started (cursor > 0), no duration on the seeded episode → Played.
    post(100, false).await;
    assert_eq!(status().await, "PLAYED");
    // Explicitly completed → Finished.
    post(0, true).await;
    assert_eq!(status().await, "FINISHED");
}

#[tokio::test]
async fn test_store_playback_with_invalid_jwt() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let fake_id = 999999;
    let token = generate_jwt_token(&fake_id.to_string());

    let _ = fake_id;
    let store_data = serde_json::json!({
        "data": { "episode_id": 1, "cursor": 100, "completed": false },
        "params": null,
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token,
            &store_data,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_store_playback_missing_body() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id: i32 = payload
        .get("user_id")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(&user_id.to_string());

    let body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/playbacks", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_store_playback_uses_jwt_user_not_body() {
    // A playback stored by user A is owned by user A and is invisible to
    // user B — the body cannot spoof another user (IDOR regression guard).
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_a: i32 = payload["user_id"].as_str().unwrap().parse().unwrap();
    let user_b: i32 = payload["admin_id"].as_str().unwrap().parse().unwrap();
    let episode_id: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();

    // User A stores a playback.
    let token_a = generate_jwt_token(&user_a.to_string());
    let store_data = serde_json::json!({
        "data": { "episode_id": episode_id, "cursor": 100, "completed": false },
        "params": null,
    });
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token_a,
            &store_data,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["data"]["user_id"].as_i64(), Some(user_a as i64));
    let playback_id = json["data"]["id"].as_i64().unwrap();

    // User B cannot read user A's playback by id.
    let token_b = generate_jwt_token(&user_b.to_string());
    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/playbacks/{}", playback_id),
            &token_b,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_store_playback_invalid_episode_id_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id: i32 = payload
        .get("user_id")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(&user_id.to_string());

    let store_data = serde_json::json!({
        "data": { "episode_id": 0, "cursor": 100, "completed": false },
        "params": null,
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token,
            &store_data,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["episode_id"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "episode_id"),
        "Episode ID must be a valid integer"
    );
}

#[tokio::test]
async fn test_store_playback_cursor_negative_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id: i32 = payload
        .get("user_id")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(&user_id.to_string());

    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let store_data = serde_json::json!({
        "data": { "episode_id": episode_id, "cursor": -1, "completed": false },
        "params": null,
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token,
            &store_data,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    // `cursor` is a u64 on the wire, so a negative is UNREPRESENTABLE —
    // serde rejects the body before the validator's range check runs. The
    // rejection is therefore the extractor's parse error (keyed "data"),
    // not a field-keyed `cursor` one (see the too-large sibling below).
    let message = field_error(&json, "data");
    assert!(
        message.contains("Failed to parse request body"),
        "expected the body-parse rejection, got: {message}"
    );
}

#[tokio::test]
async fn test_store_playback_cursor_too_large_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id: i32 = payload
        .get("user_id")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(&user_id.to_string());

    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let store_data = serde_json::json!({
        "data": { "episode_id": episode_id, "cursor": 36001, "completed": false },
        "params": null,
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/playbacks",
            &token,
            &store_data,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["cursor"].is_array(),
        "validation errors should be keyed by field name"
    );
}
