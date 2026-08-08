use axum::{Json, extract::Extension, http::HeaderMap, http::header::SET_COOKIE};
use halogen_utils::constants::{VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHENTICATED_CODE};
use halogen_wire::{LoginData, ResponseData, TokenData};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tracing::info;

use super::token;
use crate::routers::errors::ApiError;
use crate::routers::extractors::Body;
use crate::routers::middleware::AuthConfig;

use halogen_orm::user::{Column, Entity as UserEntity};

/// A valid bcrypt hash computed once. The user-not-found login path verifies a
/// password against it so a missing username costs the same as a wrong password,
/// closing the timing-based username-enumeration oracle.
static DUMMY_PASSWORD_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    bcrypt::hash("halogen-timing-equalizer", bcrypt::DEFAULT_COST).expect("hash dummy password")
});

#[axum::debug_handler]
pub async fn login(
    Extension(pool): Extension<DatabaseConnection>,
    state: Extension<AuthConfig>,
    request_headers: HeaderMap,
    Body(login_req): Body<LoginData>,
) -> Result<(HeaderMap, Json<ResponseData<TokenData>>), ApiError> {
    // `Body` already rejects a missing `data` field and runs `LoginRequestData`
    // validation (username/password length) with the standard error envelope.
    let invalid = || {
        ApiError::new(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_UNAUTHENTICATED_CODE,
            "Invalid credentials".to_string(),
        )
    };
    // Usernames are stored lowercase (every write path normalizes), so lowercase
    // the submitted one too — login is case-insensitive at the boundary.
    let username = login_req.username.to_lowercase();
    let user = match UserEntity::find()
        .filter(Column::Username.eq(&username))
        .one(&pool)
        .await
        .map_err(|_| invalid())?
    {
        Some(u) => u,
        None => {
            // Equalize timing with the wrong-password path so a missing username
            // isn't a measurably faster response (username-enumeration oracle).
            let _ = bcrypt::verify(&login_req.password, &DUMMY_PASSWORD_HASH);
            return Err(invalid());
        }
    };

    if !bcrypt::verify(&login_req.password, &user.password_hash).unwrap_or(false) {
        return Err(invalid());
    }

    // Mint the API bearer token (body) + the media cookie in one place. The
    // cookie's Secure/SameSite attributes follow the request's context.
    let secure = super::cookie::secure_cookie_context(&request_headers);
    let tokens = token::issue_tokens(
        &state.secret,
        &user.id.to_string(),
        state.expiry_secs,
        secure,
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, tokens.media_cookie);

    info!("User '{}' logged in", username);

    Ok((
        headers,
        Json(ResponseData::from_data(TokenData {
            token: tokens.api_token,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use super::super::tests::{build_test_router, setup_test_db};
    use crate::tests::harness::{field_error, json, json_body};

    #[tokio::test]
    async fn test_login_success_returns_token() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body =
            serde_json::json!({ "data": { "username": "admin_user", "password": "testadmin123" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        assert!(
            json.get("data").and_then(|d| d.get("token")).is_some(),
            "response must contain token in data field"
        );
    }

    // A successful login sets the `auth_media` cookie: HttpOnly, scoped to
    // `/api/v1`, and carrying a *media-scoped* JWT (never the API token).
    #[tokio::test]
    async fn test_login_sets_media_cookie() {
        use crate::routers::middleware::JwtClaims;
        use axum::http::header;
        use jsonwebtoken::{DecodingKey, Validation, decode};

        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body =
            serde_json::json!({ "data": { "username": "admin_user", "password": "testadmin123" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("login must set a cookie")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cookie.starts_with("auth_media="),
            "cookie name is auth_media"
        );
        assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(
            cookie.contains("Path=/api/v1;"),
            "cookie scoped to the API prefix (must reach audio + art routes)"
        );

        // The cookie value is a media-scoped JWT.
        let value = cookie
            .strip_prefix("auth_media=")
            .and_then(|s| s.split(';').next())
            .unwrap();
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        let claims = decode::<JwtClaims>(
            value,
            &DecodingKey::from_secret(b"test-secret-key-for-jwt-signing"),
            &validation,
        )
        .expect("cookie carries a valid jwt")
        .claims;
        assert!(claims.is_media(), "cookie token must be media-scoped");
    }

    /// Usernames are stored lowercase; a mixed-case submission must still match
    /// (the handler lowercases before the lookup).
    #[tokio::test]
    async fn test_login_mixed_case_username_succeeds() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body =
            serde_json::json!({ "data": { "username": "Admin_User", "password": "testadmin123" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_login_wrong_password_401() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": { "username": "admin_user", "password": "wrongpassword" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_login_nonexistent_user_401() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": { "username": "nobody", "password": "anything" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_login_missing_data_field_400() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({});
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    #[tokio::test]
    async fn test_login_empty_username_400() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": { "username": "", "password": "testadmin123" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["username"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(
            field_error(&json, "username"),
            "Username must be between 1 and 64 characters"
        );
    }

    #[tokio::test]
    async fn test_login_empty_password_400() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": { "username": "admin_user", "password": "" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["password"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(
            field_error(&json, "password"),
            "Password must be between 1 and 256 characters"
        );
    }

    #[tokio::test]
    async fn test_login_null_data_400() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": null });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    #[tokio::test]
    async fn test_login_error_has_correct_envelope_shape() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body = serde_json::json!({ "data": { "username": "admin_user", "password": "wrongpassword" } });
        let response = router
            .clone()
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = json_body(response).await;

        assert!(json.get("data").is_some(), "response must have data field");
        assert!(
            json.get("errors").is_some(),
            "response must have errors field"
        );
        assert!(
            json.get("paginator").is_some(),
            "response must have paginator field"
        );

        assert!(
            json["errors"]["request"].as_array().is_some(),
            "errors should have 'request' key"
        );
        let errors = json["errors"]["request"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "must have exactly one error");
        assert_eq!(
            errors[0]["code"].as_str().unwrap(),
            "unauthenticated",
            "error code must be 'unauthenticated'"
        );
        assert_eq!(
            errors[0]["message"].as_str().unwrap(),
            "Invalid credentials",
            "error message must be 'Invalid credentials'"
        );
    }

    /// Regression (H3): token lifetime is `expiry_minutes` MINUTES, not 60× that. A
    /// prior bug fed the seconds value into a minutes constructor. With the test
    /// config's 60-minute expiry the token must expire in ~1 hour, not ~60.
    #[tokio::test]
    async fn test_login_token_ttl_is_minutes_not_hours() {
        use crate::routers::middleware::JwtClaims;
        use jsonwebtoken::{DecodingKey, Validation, decode};
        use time::OffsetDateTime;

        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let body =
            serde_json::json!({ "data": { "username": "admin_user", "password": "testadmin123" } });
        let response = router
            .oneshot(json("POST", "/api/v1/auth/login", &body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let token = json["data"]["token"].as_str().expect("token in response");

        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = false; // inspect exp rather than enforce it
        let claims = decode::<JwtClaims>(
            token,
            &DecodingKey::from_secret(b"test-secret-key-for-jwt-signing"),
            &validation,
        )
        .expect("decode token")
        .claims;

        let now = OffsetDateTime::now_utc().unix_timestamp();
        let ttl = claims.exp as i64 - now;
        // 60-minute test config = 3600s. The 60× bug would yield ~216000s (60h).
        assert!(
            (3000..=4200).contains(&ttl),
            "token TTL should be ~3600s (1h), got {ttl}s"
        );
    }
}
