use axum::{Extension, Json};
use halogen_utils::constants::*;
use halogen_wire::{ResponseData, UserDeleteParams};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::user::user_delete;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Id};

/// Delete a user. Admin-only; an admin may not delete their own account.
pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    AdminUser(admin_id): AdminUser,
    Id(user_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    if admin_id == user_id {
        warn!("Admin '{}' attempted to delete their own account", admin_id);
        return Err(ApiError::new(
            VALIDATION_ID_FIELD,
            VALIDATION_INVALID_CODE,
            "Cannot delete yourself".to_string(),
        ));
    }

    let params = UserDeleteParams { id: user_id };
    user_delete::handle(&dbc, &params).await.map_err(|err| {
        warn!("Error deleting user: {:?}", err);
        ApiError(err)
    })?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use crate::routers::users::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_delete_user_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();

        let admin_token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/admin/users/{}", user_id),
                &admin_token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let data = json.get("data").expect("data field");
        assert!(data.is_null() || data.get("user").is_some());

        // Verify user is actually deleted (the self-or-admin GET stays unprefixed).
        let verify_response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/users/{}", user_id),
                &admin_token,
            ))
            .await
            .unwrap();

        assert_eq!(verify_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_user_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let fake_id = i32::MAX.to_string();

        let admin_token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/admin/users/{}", fake_id),
                &admin_token,
            ))
            .await
            .unwrap();

        let status = response.status();
        let json = json_body(response).await;
        println!("JSON: {}", serde_json::to_string_pretty(&json).unwrap());

        assert_eq!(status, StatusCode::NOT_FOUND);

        assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "exists");
        assert_eq!(field_error(&json, "id"), "User not found");
    }

    #[tokio::test]
    async fn test_delete_user_self_deletion_prevented() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/admin/users/{}", admin_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "invalid");
        assert_eq!(field_error(&json, "id"), "Cannot delete yourself");
    }

    #[tokio::test]
    async fn test_delete_user_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed("DELETE", "/api/v1/admin/users/abc", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "parsing");
        assert_eq!(
            field_error(&json, "id"),
            "Invalid ID format: expected integer, got 'abc'"
        );
    }

    #[tokio::test]
    async fn test_delete_user_zero_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed("DELETE", "/api/v1/admin/users/0", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["id"].is_array(),
            "the Id extractor rejection is keyed under \"id\""
        );
        assert_eq!(
            field_error(&json, "id"),
            "Invalid ID: must be a positive integer, got '0'"
        );
    }

    #[tokio::test]
    async fn test_delete_other_user_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        // A non-admin may not delete anyone — the AdminUser gate returns 403
        // before the self-delete check is even reached.
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/admin/users/{}", admin_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
