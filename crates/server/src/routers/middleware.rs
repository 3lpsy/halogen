use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};

use crate::routers::errors::ApiError;
use halogen_orm::user::{Column, Entity as UserEntity};
use halogen_utils::constants::{
    VALIDATION_AUTHTOKEN_FIELD, VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE,
    VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHENTICATED_CODE, VALIDATION_UNAUTHORIZED_CODE,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use tower::{Layer, Service};
use tracing::error;

/// Scope claim value marking a **media-only** token. Such a token authenticates
/// the audio endpoint via the `auth_media` cookie and is *rejected* by the API
/// bearer middleware, so a leaked media credential can't touch the API (and the
/// API token can't stream media).
pub const MEDIA_SCOPE: &str = "media";

/// Scope claim value marking a **WebSocket ticket** — a short-lived credential
/// minted by `POST /ws-ticket` and passed as the `?ticket=` query param on the
/// `/ws` upgrade (browsers can't set an `Authorization` header on a WebSocket).
/// Scope-locked like [`MEDIA_SCOPE`]: rejected by the API bearer middleware, and
/// the WS handshake accepts *only* this scope, so neither token type works on the
/// other path.
pub const WS_SCOPE: &str = "ws";

/// JWT claims used in the token.
///
/// `scope` differentiates credentials: `None` (default) is a normal full-access
/// API token; `Some("media")` is a media-only token (see [`MEDIA_SCOPE`]);
/// `Some("ws")` is a WebSocket ticket (see [`WS_SCOPE`]). The field is
/// `#[serde(default)]` so older tokens without it decode as API tokens.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl JwtClaims {
    /// A full-access API token (no scope).
    pub fn api(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: None,
        }
    }

    /// A media-only token (`scope = "media"`).
    pub fn media(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: Some(MEDIA_SCOPE.to_string()),
        }
    }

    /// A WebSocket ticket (`scope = "ws"`).
    pub fn ws(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: Some(WS_SCOPE.to_string()),
        }
    }

    /// True when this is a normal full-access API token (no scope). The inverse of
    /// "any scope-locked credential" — use this (not `!is_media()`) wherever only a
    /// real API token is acceptable, so a `ws` ticket isn't silently let through.
    pub fn is_api(&self) -> bool {
        self.scope.is_none()
    }

    /// True when this is a media-scoped token.
    pub fn is_media(&self) -> bool {
        self.scope.as_deref() == Some(MEDIA_SCOPE)
    }

    /// True when this is a WebSocket ticket.
    pub fn is_ws(&self) -> bool {
        self.scope.as_deref() == Some(WS_SCOPE)
    }
}

/// Authenticated user info extracted from a valid JWT.
#[derive(Debug, Clone)]
pub struct JwtAuth {
    pub user_id: String,
    pub username: String,
    pub is_admin: bool,
}

/// Auth configuration (JWT secret + expiry). Consumed by [`JwtAuthLayer`] AND
/// handed to the login/refresh/media handlers as an `Extension` — it's the same
/// two values, so one struct serves both.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub secret: String,
    pub expiry_secs: u64,
}

/// JwtAuthLayer holds both the db connection and JWT configuration.
#[derive(Clone)]
pub struct JwtAuthLayer {
    dbc: DatabaseConnection,
    config: AuthConfig,
}

impl JwtAuthLayer {
    pub fn new(dbc: DatabaseConnection, config: AuthConfig) -> Self {
        Self { dbc, config }
    }
}

impl<S> Layer<S> for JwtAuthLayer {
    type Service = JwtAuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        JwtAuthMiddleware {
            inner,
            dbc: self.dbc.clone(),
            config: self.config.clone(),
        }
    }
}

pub struct JwtAuthMiddleware<S> {
    inner: S,
    dbc: sea_orm::DatabaseConnection,
    config: AuthConfig,
}

impl<S> Clone for JwtAuthMiddleware<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            dbc: self.dbc.clone(),
            config: self.config.clone(),
        }
    }
}

impl<S, B> Service<axum::http::Request<B>> for JwtAuthMiddleware<S>
where
    S: Service<axum::http::Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: axum::body::HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: Into<axum::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: axum::http::Request<B>) -> Self::Future {
        let dbc = self.dbc.clone();
        let config = self.config.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let token = match extract_bearer(req.headers()) {
                Some(t) => t,
                None => {
                    return Ok(auth_error(
                        StatusCode::UNAUTHORIZED,
                        "Missing or malformed Authorization header",
                    ));
                }
            };

            let jwt_auth = match validate_and_lookup(&token, &config.secret, &dbc).await {
                Ok(auth) => auth,
                Err(status) => {
                    let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
                        "Internal error during authentication"
                    } else {
                        "Invalid or expired token"
                    };
                    return Ok(auth_error(status, message));
                }
            };

            req.extensions_mut().insert(jwt_auth);

            inner.call(req).await
        })
    }
}

/// The `Bearer <token>` value from the `Authorization` header, if present and
/// well-formed. Shared with the media-auth path so both parse the header the same
/// way.
pub(crate) fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth_header.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Build a standard `ResponseData` error envelope (via [`ApiError`]) for an auth
/// failure, instead of an ad-hoc `{"error":"..."}` body — matches every other
/// API. The `ApiError` code is derived from `status` so the response keeps it.
fn auth_error(status: StatusCode, message: &str) -> Response {
    // This is the API *bearer* path, so a 401 is keyed to the `authtoken`
    // transport (the cookie path keys `authcookie`). field = WHERE, code = WHY:
    //   401 → no/invalid credentials (`unauthenticated`)
    //   403 → authenticated but not permitted (`unauthorized`)
    //   500 → unexpected failure in the auth DB lookup (`database`/`panic`)
    let (field, code) = match status {
        StatusCode::FORBIDDEN => (VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHORIZED_CODE),
        StatusCode::INTERNAL_SERVER_ERROR => (VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE),
        _ => (VALIDATION_AUTHTOKEN_FIELD, VALIDATION_UNAUTHENTICATED_CODE),
    };
    // `ApiError` has both an inherent `into_response::<T>()` and an `IntoResponse`
    // impl; the inherent one shadows the trait, so spell out `T = ()` (no data
    // payload — the body is the validation-error envelope `ResponseData<()>`).
    ApiError::new(field, code, message.to_string()).into_response::<()>()
}

async fn validate_and_lookup(
    token: &str,
    secret: &str,
    dbc: &sea_orm::DatabaseConnection,
) -> Result<JwtAuth, StatusCode> {
    let claims = crate::routers::auth::token::decode_claims(token, secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // A media-only credential or a WebSocket ticket must never authenticate an
    // API route — each is scope-locked to its own transport.
    if claims.is_media() || claims.is_ws() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let user = get_user_data(dbc, &claims.sub).await?;

    Ok(JwtAuth {
        user_id: user.id.to_string(),
        username: user.username,
        is_admin: user.is_admin,
    })
}

async fn get_user_data(
    dbc: &sea_orm::DatabaseConnection,
    id: &str,
) -> Result<halogen_orm::user::Model, StatusCode> {
    let user_id: i32 = id.parse().map_err(|e| {
        error!("Invalid user ID for lookup: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let user = UserEntity::find()
        .filter(Column::Id.eq(user_id))
        .one(dbc)
        .await
        .map_err(|e| {
            error!("Failed to look up user: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    user.ok_or(StatusCode::UNAUTHORIZED)
}

#[cfg(test)]
mod tests {
    use crate::routers::middleware::JwtClaims;
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use jsonwebtoken::{EncodingKey, Header, encode};
    use tower::ServiceExt;

    /// The same secret `build_test_router` configures via the test `Config`.
    const TEST_SECRET: &[u8] = b"test-secret-key-for-jwt-signing";

    /// Encode a JWT against an arbitrary secret with an arbitrary expiry.
    ///
    /// Mirrors `generate_jwt_token` but lets each test control the secret and
    /// `exp` so we can exercise the wrong-secret and expired paths.
    fn encode_token(sub: &str, exp: usize, secret: &[u8]) -> String {
        let claims = JwtClaims::api(sub.to_string(), exp);
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .expect("encode token")
    }

    /// Fire a `GET /playbacks` (a protected route) and return the status. When
    /// `auth` is `Some`, it is sent verbatim as the `Authorization` header.
    async fn request_protected(auth: Option<&str>) -> StatusCode {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let mut builder = Request::builder().method("GET").uri("/api/v1/playbacks");
        if let Some(value) = auth {
            builder = builder.header("Authorization", value);
        }
        let request = builder.body(Body::empty()).unwrap();

        router.oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn test_missing_authorization_header_is_401() {
        // `extract_bearer` returns None → "Missing or malformed Authorization header".
        assert_eq!(request_protected(None).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_header_without_bearer_prefix_is_401() {
        // No `Bearer ` prefix → `strip_prefix` returns None → 401.
        assert_eq!(
            request_protected(Some("token-without-prefix")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_bearer_with_garbage_token_is_401() {
        // Present prefix but undecodable token → `decode` fails → 401.
        assert_eq!(
            request_protected(Some("Bearer not-a-real-jwt")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_expired_token_is_401() {
        // Right secret, but `exp` in the past → `validate_exp` rejects it → 401.
        let past_exp = 1_000_000_000usize; // 2001-09-09, well in the past.
        let token = encode_token("1", past_exp, TEST_SECRET);
        assert_eq!(
            request_protected(Some(&format!("Bearer {token}"))).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_wrong_secret_token_is_401() {
        // Valid shape and future expiry, but signed with a different secret →
        // signature verification fails → 401.
        let future_exp = 4_000_000_000usize; // 2096, far future.
        let token = encode_token("1", future_exp, b"the-wrong-secret");
        assert_eq!(
            request_protected(Some(&format!("Bearer {token}"))).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_valid_token_for_nonexistent_user_is_401() {
        // Token decodes fine, but the `sub` user is absent from the DB. The
        // lookup's `ok_or(StatusCode::UNAUTHORIZED)` turns the missing row into
        // a 401 (not a 500/404).
        let token = generate_jwt_token("999999");
        assert_eq!(
            request_protected(Some(&format!("Bearer {token}"))).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn test_media_scoped_token_rejected_on_api_route() {
        // A correctly-signed, unexpired token whose `sub` is a real user but
        // carries `scope=media` must NOT authenticate an API route — the media
        // credential is for the audio endpoint only.
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let future_exp = 4_000_000_000usize;
        let media = encode(
            &Header::default(),
            &JwtClaims::media(user_id.to_string(), future_exp),
            &EncodingKey::from_secret(TEST_SECRET),
        )
        .expect("encode media token");

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/playbacks")
                    .header("Authorization", format!("Bearer {media}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_valid_token_for_seeded_user_is_200() {
        // Positive control: a correctly-signed, unexpired token whose `sub`
        // matches a seeded user passes the middleware and reaches the handler.
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/playbacks")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
