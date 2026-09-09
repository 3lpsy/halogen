use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, json_body};
use axum::http::StatusCode;
use halogen_orm::user::ActiveModel as UserActiveModel;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use tower::ServiceExt;

#[tokio::test]
async fn test_get_episode_returns_episode() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{}", episode_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let ep = json.get("data").expect("data");
    assert_eq!(ep.get("title").unwrap().as_str().unwrap(), "Test Episode 1");
}

/// A user who is NOT subscribed to the episode's podcast must not be able to
/// read it by id — the subscription read-guard returns 404 (hiding existence),
/// the single-episode analogue of the scoped list route.
#[tokio::test]
async fn test_get_episode_hidden_from_non_subscriber() {
    let (_root, dbc, payload) = setup_test_db().await;

    // A second, real user subscribed to nothing. The auth middleware looks the
    // user up by id, so the row must exist for the token to authenticate — the
    // 404 must come from the subscription guard, not a missing user.
    let outsider_id = i32::MAX - 100;
    UserActiveModel {
        id: Set(outsider_id),
        username: Set("outsider".to_string()),
        password_hash: Set("x".to_string()),
        is_admin: Set(false),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&dbc)
    .await
    .expect("insert outsider user");

    let episode_id = payload.get("episode_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(&outsider_id.to_string());
    let router = build_test_router(dbc);

    let response = router
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{}", episode_id),
            &token,
        ))
        .await
        .unwrap();

    // 404, not 403 — the read-guard hides existence from outsiders.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = json_body(response).await;
    let code = json
        .get("errors")
        .and_then(|e| e.get("id"))
        .and_then(|i| i.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("code"))
        .and_then(|c| c.as_str())
        .expect("error code present");
    assert_eq!(code, "exists", "non-subscriber must not read the episode");
}

#[tokio::test]
async fn test_get_episode_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let fake_id = i32::MAX.to_string();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{}", fake_id),
            &token,
        ))
        .await
        .unwrap();

    let status = response.status();
    let json = json_body(response).await;
    println!("JSON: {}", serde_json::to_string_pretty(&json).unwrap());

    assert_eq!(status, StatusCode::NOT_FOUND);

    let errors_array = json
        .get("errors")
        .unwrap()
        .get("id")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        errors_array[0].get("code").unwrap().as_str().unwrap(),
        "exists"
    );
    // The subscription read-guard now runs before the handler, so a missing
    // episode 404s from the guard ("Episode not found") rather than the
    // handler's "episode does not exist". Same status + `exists` code.
    assert_eq!(
        errors_array[0].get("message").unwrap().as_str().unwrap(),
        "Episode not found"
    );
}

#[tokio::test]
async fn test_get_episode_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/episodes/99999999999999", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    let errors_array = json
        .get("errors")
        .unwrap()
        .get("id")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        errors_array[0].get("code").unwrap().as_str().unwrap(),
        "parsing"
    );
    assert_eq!(
        errors_array[0].get("message").unwrap().as_str().unwrap(),
        "Invalid ID format: expected integer, got '99999999999999'"
    );
}
