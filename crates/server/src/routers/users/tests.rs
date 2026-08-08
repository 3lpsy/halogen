use tracing::debug;

use crate::tests::harness::seed_admin_and_user;
use halogen_fixture::test_support::TestRoot;

pub use crate::tests::harness::{build_test_router, generate_jwt_token};

/// Create a test SQLite database with seeded users using SeaORM entities.
/// Returns (TestRoot, DatabaseConnection, serde_json::Value) so the TestRoot
/// stays alive for the lifetime of the test (preventing premature cleanup).
pub async fn setup_test_db() -> (TestRoot, sea_orm::DatabaseConnection, serde_json::Value) {
    let (mut root, dbc) = crate::tests::harness::new_test_db("test_users").await;

    // This suite uses low, fixed ids (1, 2) — several tests assert on them.
    let admin_id = 1;
    let user_id = 2;
    seed_admin_and_user(&dbc, admin_id, user_id).await;

    let payload = serde_json::json!({
        "admin_id": admin_id.to_string(),
        "admin_password": "testadmin123",
        "user_id": user_id.to_string(),
        "user_password": "testuser123",
    });

    root.mark_success();
    (root, dbc, payload)
}

#[tokio::test]
async fn debug_fetch_error() {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let (_root, dbc, payload) = setup_test_db().await;

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();

    // Try the exact same query as fetch_user_by_id
    let result = halogen_orm::user::Entity::find()
        .filter(halogen_orm::user::Column::Id.eq(user_id.parse::<i32>().unwrap()))
        .one(&dbc)
        .await;

    match &result {
        Ok(Some(u)) => debug!("Fetched user: {:?}", u.username),
        Ok(None) => panic!("User not found in database!"),
        Err(e) => panic!("Database error: {:?}", e),
    }

    assert!(result.is_ok(), "Query should succeed");
    assert!(result.unwrap().is_some(), "User should exist");
}

#[tokio::test]
async fn debug_handler_direct() {
    use axum::body::to_bytes;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let admin_token = generate_jwt_token(admin_id);

    debug!("admin_id: {}, user_id: {}", admin_id, user_id);

    // Test DELETE /users/{user_id}
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/users/{}", user_id))
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    debug!("Response status: {}", response.status());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    debug!("Response body: {}", String::from_utf8_lossy(&body));
}
