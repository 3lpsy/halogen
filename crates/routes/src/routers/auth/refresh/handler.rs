use axum::{Json, extract::Extension, http::HeaderMap, http::header::SET_COOKIE};
use halogen_orm::user::Entity as UserEntity;
use halogen_utils::constants::{
    VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD,
    VALIDATION_UNAUTHENTICATED_CODE,
};
use halogen_wire::{ResponseData, TokenData};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::super::token;
use crate::routers::errors::ApiError;
use crate::routers::extractors::Body;
use crate::routers::middleware::AuthConfig;

#[derive(Debug, Validate, Serialize, Deserialize)]
pub struct RefreshRequestData {
    #[validate(length(min = 1, max = 4096, message = "Token is required"))]
    pub token: String,
}

pub async fn refresh(
    Extension(dbc): Extension<DatabaseConnection>,
    state: Extension<AuthConfig>,
    request_headers: HeaderMap,
    Body(refresh_req): Body<RefreshRequestData>,
) -> Result<(HeaderMap, Json<ResponseData<TokenData>>), ApiError> {
    // `Body` enforces the present `data` field and runs `RefreshRequestData`
    // validation (non-empty token) before the JWT decode below.
    let invalid_token = || {
        ApiError::new(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_UNAUTHENTICATED_CODE,
            "Invalid or expired token".to_string(),
        )
    };

    let claims =
        token::decode_claims(&refresh_req.token, &state.secret).map_err(|_| invalid_token())?;

    // Only a normal full-access API token may be refreshed. A media-only credential OR a WebSocket ticket (both
    // scope-locked to their own transport — and the WS ticket rides in a URL query string, where it's prone to
    // leak) must never be upgradable into a full-access token. Checking `is_api()` rather than just
    // `!is_media()` is what closes the `ws`-ticket escalation path.
    if !claims.is_api() {
        return Err(invalid_token());
    }

    // The subject must still exist: a deleted user's (still-unexpired) token must
    // not be refreshable into a fresh session. Mirrors the API middleware, which
    // re-resolves the user on every request.
    let user_id: i32 = claims.sub.parse().map_err(|_| invalid_token())?;
    if UserEntity::find_by_id(user_id)
        .one(&dbc)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Authentication user lookup failed");
            ApiError::new(
                VALIDATION_DATABASE_FIELD,
                VALIDATION_PANIC_CODE,
                "Authentication is temporarily unavailable".to_string(),
            )
        })?
        .is_none()
    {
        return Err(invalid_token());
    }

    // Reissue the API token + media cookie (so playback survives a refresh). The
    // cookie's Secure/SameSite attributes follow the request's context.
    let secure = super::super::cookie::secure_cookie_context(&request_headers);
    let tokens = token::issue_tokens(&state.secret, &claims.sub, state.expiry_secs, secure)?;

    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, tokens.media_cookie);

    Ok((
        headers,
        Json(ResponseData::from_data(TokenData {
            token: tokens.api_token,
        })),
    ))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
