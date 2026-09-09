use crate::routers::users::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

fn body(username: &str, password: &str, is_admin: bool) -> serde_json::Value {
    serde_json::json!({
        "data": {
            "username": username,
            "password": password,
            "password_confirm": password,
            "is_admin": is_admin,
        }
    })
}

/// Regression: a sentinel row at `i32::MAX - 1` (the dev seed's second
/// user) must not make the allocator hand out `i32::MAX` itself — that
/// collided with the seeded admin and 400'd every user creation on a
/// dev-seeded server.
#[tokio::test]
async fn test_store_user_with_sentinel_neighbor_allocates_below_region() {
    let (_root, dbc, payload) = setup_test_db().await;
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    // Plant a second sentinel right below the admin, like the dev seed.
    halogen_fixture::user::seed_password_user(
        &dbc,
        i32::MAX - 1,
        "sentinel-neighbor",
        "password123",
        false,
    )
    .await
    .expect("seeding the sentinel neighbor");
    let router = build_test_router(dbc);
    let token = generate_jwt_token(admin_id);

    let response = router
        .oneshot(authed_json(
            "POST",
            "/api/v1/admin/users",
            &token,
            &body("allocated", "password123", false),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let id = json["data"]["id"].as_i64().unwrap();
    assert!(
        id < (i32::MAX - 1024) as i64,
        "allocated id {id} must stay below the sentinel region"
    );
}

#[tokio::test]
async fn test_store_user_as_admin_creates_and_lowercases() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .oneshot(authed_json(
            "POST",
            "/api/v1/admin/users",
            &token,
            &body("NewUser", "password123", false),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let data = json.get("data").unwrap();
    assert_eq!(data.get("username").unwrap(), "newuser");
    assert_eq!(data.get("is_admin").unwrap(), false);
}

#[tokio::test]
async fn test_store_user_duplicate_username_is_validation_error() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let first = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/admin/users",
            &token,
            &body("dupe", "password123", false),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = router
        .oneshot(authed_json(
            "POST",
            "/api/v1/admin/users",
            &token,
            &body("Dupe", "password123", false),
        ))
        .await
        .unwrap();
    // The shared DB-error mapping turns the unique-index violation into a
    // 409 whose envelope carries a `unique` error (keyed under the generic
    // `request` field, naming the column in the message).
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let json = json_body(second).await;
    let msg = field_error(&json, "request");
    assert!(
        msg.contains("username") && msg.contains("unique"),
        "duplicate maps to a unique-username error, got: {msg}"
    );
}

#[tokio::test]
async fn test_store_user_requires_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    // `setup_test_db` seeds a second, non-admin user under "user_id".
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(authed_json(
            "POST",
            "/api/v1/admin/users",
            &token,
            &body("nope", "password123", false),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
