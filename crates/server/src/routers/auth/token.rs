//! JWT decode + token-pair issuance — the single owner of the HS256 validation
//! config and the login/refresh credential minting.
//!
//! [`decode_claims`] owns the `DecodingKey` + `Validation` setup for every caller
//! (the API middleware, the media-auth path, and token refresh) and
//! [`issue_tokens`] owns the api+media encode/cookie dance for both `login` and
//! `refresh`, so the algorithm choice and expiry policy can't drift between paths.

use axum::http::HeaderValue;
use halogen_utils::constants::{VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD};
use jsonwebtoken::errors::Error as JwtError;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use time::{Duration, OffsetDateTime};

use super::cookie::set_media_cookie;
use crate::routers::errors::ApiError;
use crate::routers::middleware::JwtClaims;

/// Decode + verify a HS256 JWT, enforcing expiry. The one place the decode config
/// (algorithm + `validate_exp`) lives — shared by the API bearer middleware, the
/// media-auth path, and token refresh, so they can't disagree on what "valid"
/// means.
pub fn decode_claims(token: &str, secret: &str) -> Result<JwtClaims, JwtError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_exp = true;
    decode::<JwtClaims>(token, &key, &validation).map(|d| d.claims)
}

/// The freshly-minted credential pair for a session: the API bearer token (for
/// the response body) and a ready `Set-Cookie` value carrying the media token.
pub struct IssuedTokens {
    pub api_token: String,
    pub media_cookie: HeaderValue,
}

/// Mint a full-access API token plus the media-scoped cookie for `sub`, both
/// expiring in `expiry_secs`. The media token rides the `auth_media` cookie so
/// `<audio>`/`<img>` requests authenticate without a header. `secure_cookie`
/// selects the cookie's `Secure`/`SameSite` attributes from the request's
/// context (see `cookie::secure_cookie_context`).
pub fn issue_tokens(
    secret: &str,
    sub: &str,
    expiry_secs: u64,
    secure_cookie: bool,
) -> Result<IssuedTokens, ApiError> {
    let now = OffsetDateTime::now_utc();
    // `.min(i64::MAX)` so a pathological `expiry_secs` can't wrap the `i64` cast
    // negative (which would mint an already-expired token → lockout).
    let exp_ts =
        (now + Duration::seconds(expiry_secs.min(i64::MAX as u64) as i64)).unix_timestamp();
    let exp = exp_ts as usize;
    let key = EncodingKey::from_secret(secret.as_bytes());

    let api_token = encode(
        &Header::default(),
        &JwtClaims::api(sub.to_string(), exp),
        &key,
    )
    .map_err(|_| token_error())?;
    let media_token = encode(
        &Header::default(),
        &JwtClaims::media(sub.to_string(), exp),
        &key,
    )
    .map_err(|_| token_error())?;

    let max_age = (exp_ts - now.unix_timestamp()).max(0) as u64;
    Ok(IssuedTokens {
        api_token,
        media_cookie: set_media_cookie(&media_token, max_age, secure_cookie),
    })
}

/// Mint a short-lived WebSocket ticket (`scope = "ws"`) for `sub`, expiring in
/// `ttl_secs`. Stateless: it's a normal HS256 JWT signed with the same secret, so
/// the `/ws` handshake verifies it without server-side state (survives restarts,
/// multi-instance safe). Kept deliberately short-lived since it travels in the
/// upgrade URL's query string; the client re-mints on every (re)connect.
pub fn mint_ws_ticket(secret: &str, sub: &str, ttl_secs: u64) -> Result<String, ApiError> {
    let exp_ts = (OffsetDateTime::now_utc() + Duration::seconds(ttl_secs as i64)).unix_timestamp();
    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(
        &Header::default(),
        &JwtClaims::ws(sub.to_string(), exp_ts as usize),
        &key,
    )
    .map_err(|_| token_error())
}

fn token_error() -> ApiError {
    ApiError::new(
        VALIDATION_REQUEST_FIELD,
        VALIDATION_PANIC_CODE,
        "Failed to create token".to_string(),
    )
}
