use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};

use halogen_orm::user::{Column, Entity as UserEntity};
use halogen_utils::constants::{
    VALIDATION_AUTHTOKEN_FIELD, VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE,
    VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHENTICATED_CODE, VALIDATION_UNAUTHORIZED_CODE,
};
use halogen_wire_meta::error::ApiError;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tower::{Layer, Service};
use tracing::error;

use crate::{AuthConfig, JwtAuth};

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
pub fn extract_bearer(headers: &HeaderMap) -> Option<String> {
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
    // This is the API *bearer* path, so a 401 is keyed to the `authtoken` transport (the cookie path keys
    // `authcookie`). field = WHERE, code = WHY: 401 → no/invalid credentials (`unauthenticated`) 403 →
    // authenticated but not permitted (`unauthorized`) 500 → unexpected failure in the auth DB lookup
    // (`database`/`panic`)
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
    let claims =
        crate::token::decode_claims(token, secret).map_err(|_| StatusCode::UNAUTHORIZED)?;

    // A media-only credential or a WebSocket ticket must never authenticate an
    // API route — each is scope-locked to its own transport.
    if !claims.is_api() {
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
