use sea_orm::ActiveModelTrait;

use crate::tests::harness::user_model;
use halogen_fixture::test_support::TestRoot;

pub use crate::tests::harness::{build_test_router, generate_jwt_token};

/// Create a test SQLite database with a seeded admin user using SeaORM entities.
pub async fn setup_test_db() -> (TestRoot, sea_orm::DatabaseConnection, serde_json::Value) {
    let (mut root, dbc) = crate::tests::harness::new_test_db("test_auth").await;

    let password = "testadmin123";
    let id = i32::MAX;
    user_model(id, "admin_user", password, true)
        .insert(&dbc)
        .await
        .expect("insert admin");

    let payload = serde_json::json!({ "password": password, "id": id });
    root.mark_success();
    (root, dbc, payload)
}
