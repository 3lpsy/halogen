//! Media auth accepts normal API bearer tokens or media-scoped auth_media cookies, rejecting cross-use. Validate
//! signature and expiry, then return the subject. Routes separately resolve the DB admin flag and authorize resource
//! access; this helper performs no user lookup.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::routers::auth::cookie::read_media_cookie;
use crate::routers::auth::token::decode_claims;
use crate::routers::middleware::{AuthConfig, extract_bearer};

/// The authenticated user id when the request carries a valid, scope-correct
/// media credential, else `None`. `Some(id)` means *authenticated*, not yet
/// *authorized* for a given episode — the caller still runs the subscription
/// guard. Mirrors the API middleware, which parses the same `sub` claim.
pub fn media_request_user(headers: &HeaderMap, auth: &AuthConfig) -> Option<i32> {
    let decode = |token: &str| decode_claims(token, &auth.secret).ok();

    // Bearer presence is decided BEFORE validity: an `Authorization` header always
    // takes the bearer arm, so a wrong-scope bearer is rejected without falling
    // back to the cookie.
    let claims = match (extract_bearer(headers), read_media_cookie(headers)) {
        // Bearer wins when present: must be a NORMAL full-access API token (no
        // scope). `is_api()` (not `!is_media()`) also rejects a `ws` ticket, which
        // must not authenticate the media endpoints.
        (Some(token), _) => decode(&token).filter(|c| c.is_api()),
        // Cookie path: must be a media-scoped token.
        (None, Some(token)) => decode(&token).filter(|c| c.is_media()),
        (None, None) => None,
    }?;
    // A non-numeric `sub` can't identify a user → treat as unauthorized.
    claims.sub.parse::<i32>().ok()
}

/// The authenticated media user id, or the standard `401` response to return
/// verbatim. Centralizes the one error body the media routes emit outside the
/// `ApiError` envelope — `<audio>`/`<img>` requests can't render JSON anyway, so
/// the three media handlers shared this literal.
#[allow(clippy::result_large_err)] // Return Axum's response directly without an extra allocation.
pub fn media_user_or_401(headers: &HeaderMap, auth: &AuthConfig) -> Result<i32, Response> {
    media_request_user(headers, auth).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            r#"{"error":"Invalid or expired token"}"#,
        )
            .into_response()
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
