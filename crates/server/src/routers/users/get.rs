use axum::{Json, extract::Extension};
use halogen_wire::{ResponseData, UserData, UserShowParams};
use sea_orm::DatabaseConnection;

use crate::handlers::user::user_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

/// Fetch a single user by ID. A non-admin may read only their own record.
pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(user_id): Id,
) -> Result<Json<ResponseData<UserData>>, ApiError> {
    guards::require_self_or_admin(actor, user_id)?;

    let params = UserShowParams {
        id: Some(user_id),
        username: None,
    };

    let user_data = user_get::handle(&dbc, &params).await.map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(user_data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::users::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_user_returns_user() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", format!("/api/v1/users/{}", user_id), &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let user = json.get("data").expect("data");
        assert_eq!(
            user.get("username").unwrap().as_str().unwrap(),
            "regular_user"
        );
    }

    #[tokio::test]
    async fn test_get_user_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);
        let fake_id = i32::MAX.to_string();

        let response = router
            .clone()
            .oneshot(authed("GET", format!("/api/v1/users/{}", fake_id), &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let json = json_body(response).await;
        assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "exists");
        assert_eq!(field_error(&json, "id"), "user does not exist");
    }

    #[tokio::test]
    async fn test_get_user_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/users/abc", &token))
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
    async fn test_get_user_zero_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/users/0", &token))
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
    async fn test_get_other_user_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        // Non-admin requesting a *different* user's record → 403.
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", format!("/api/v1/users/{}", admin_id), &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_get_any_user_allowed_for_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        // Admin may read any user's record.
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed("GET", format!("/api/v1/users/{}", user_id), &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
