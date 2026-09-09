use axum::{Json, extract::Extension, http::HeaderMap, http::header::SET_COOKIE};
use halogen_utils::constants::{
    VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD,
    VALIDATION_UNAUTHENTICATED_CODE,
};
use halogen_wire::{LoginData, ResponseData, TokenData};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tracing::info;

use super::super::token;
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
        .map_err(|error| {
            tracing::error!(%error, "Authentication user lookup failed");
            ApiError::new(
                VALIDATION_DATABASE_FIELD,
                VALIDATION_PANIC_CODE,
                "Authentication is temporarily unavailable".to_string(),
            )
        })? {
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
    let secure = super::super::cookie::secure_cookie_context(&request_headers);
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
#[path = "tests.rs"]
mod tests;
