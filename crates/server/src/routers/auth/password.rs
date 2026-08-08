use axum::{Extension, Json};
use halogen_wire::{PasswordChangeData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::user::password_change;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Body};

/// `POST /auth/password` — change the authenticated user's own password.
///
/// The account is taken from the bearer token (`AuthUserId`), never the body, so
/// a user can only ever change their own password. The body carries the current
/// password (re-verified by the handler) plus the new password and its
/// confirmation; `Body<PasswordChangeData>` has already enforced the 8-char
/// minimum and that the two new entries match.
pub async fn change_password(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Body(data): Body<PasswordChangeData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    password_change::handle(&dbc, user_id, &data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    use super::super::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{field_error, json, json_body};

    /// POST /api/v1/auth/password with the given body and optional bearer token.
    async fn change(
        router: &axum::Router,
        token: Option<&str>,
        body: &str,
    ) -> axum::http::Response<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/password")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    async fn login(router: &axum::Router, password: &str) -> StatusCode {
        let body =
            serde_json::json!({ "data": { "username": "admin_user", "password": password } });
        router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn test_change_password_success_and_relogin() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(&i32::MAX.to_string());

        let response = change(
            &router,
            Some(&token),
            r#"{"data":{"current_password":"testadmin123","new_password":{"password":"newpass1234","password_confirm":"newpass1234"}}}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        // The change actually took: the new password logs in, the old one does not.
        assert_eq!(login(&router, "newpass1234").await, StatusCode::OK);
        assert_eq!(
            login(&router, "testadmin123").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_change_password_wrong_current_is_401() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(&i32::MAX.to_string());

        let response = change(
            &router,
            Some(&token),
            r#"{"data":{"current_password":"not-my-password","new_password":{"password":"newpass1234","password_confirm":"newpass1234"}}}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let json = json_body(response).await;
        assert_eq!(
            field_error(&json, "current_password"),
            "Current password is incorrect"
        );

        // The password was not changed.
        assert_eq!(login(&router, "testadmin123").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_change_password_confirmation_mismatch_is_400() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(&i32::MAX.to_string());

        let response = change(
            &router,
            Some(&token),
            r#"{"data":{"current_password":"testadmin123","new_password":{"password":"newpass1234","password_confirm":"different99"}}}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert!(
            json["errors"]["new_password.password_confirm"].is_array(),
            "mismatch should be keyed by the nested field name"
        );
    }

    #[tokio::test]
    async fn test_change_password_too_short_is_400() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(&i32::MAX.to_string());

        let response = change(
            &router,
            Some(&token),
            r#"{"data":{"current_password":"testadmin123","new_password":{"password":"short","password_confirm":"short"}}}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert_eq!(
            field_error(&json, "new_password.password"),
            "Password must be between 8 and 256 characters long"
        );
    }

    #[tokio::test]
    async fn test_change_password_requires_auth_401() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        // No bearer token → the JWT layer rejects before the handler.
        let response = change(
            &router,
            None,
            r#"{"data":{"current_password":"testadmin123","new_password":{"password":"newpass1234","password_confirm":"newpass1234"}}}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
