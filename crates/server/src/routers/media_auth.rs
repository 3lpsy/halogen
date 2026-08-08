//! Shared authorization for the media endpoints (audio streaming + artwork).
//!
//! Two auth formats, each scope-locked to its transport:
//! - **`Authorization: Bearer`** carrying a *normal* (non-media) API token —
//!   programmatic fetches (device downloads, art revalidation). A media token
//!   as bearer is rejected: a leaked streaming-cookie value must not become an
//!   API credential.
//! - **`auth_media` cookie** carrying a *media-scoped* token — `<audio src>` /
//!   `<img src>` requests, which can't set headers but do send cookies. The
//!   full-access API token in the cookie is rejected (cross-use guard).
//!
//! Signature + expiry validation only; no DB user lookup here — the signed token
//! is the authority (mirrors the API middleware). The decoded `sub` is returned
//! so callers can authorize the user against the requested episode (subscription
//! check); resolving the admin flag is a separate DB step in the route.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::routers::auth::cookie::read_media_cookie;
use crate::routers::auth::token::decode_claims;
use crate::routers::middleware::{AuthConfig, extract_bearer};

/// The authenticated user id when the request carries a valid, scope-correct
/// media credential, else `None`. `Some(id)` means *authenticated*, not yet
/// *authorized* for a given episode — the caller still runs the subscription
/// guard. Mirrors the API middleware, which parses the same `sub` claim.
pub(crate) fn media_request_user(headers: &HeaderMap, auth: &AuthConfig) -> Option<i32> {
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
pub(crate) fn media_user_or_401(headers: &HeaderMap, auth: &AuthConfig) -> Result<i32, Response> {
    media_request_user(headers, auth).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            r#"{"error":"Invalid or expired token"}"#,
        )
            .into_response()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::auth::cookie::MEDIA_COOKIE;
    use crate::routers::middleware::JwtClaims;
    use axum::http::header::COOKIE;
    use jsonwebtoken::{EncodingKey, Header, encode};

    const SECRET: &[u8] = b"media-auth-test-secret";
    /// Far-future expiry so `validate_exp` always passes.
    const FUTURE_EXP: usize = 4_000_000_000;

    fn auth_state() -> AuthConfig {
        AuthConfig {
            secret: String::from_utf8(SECRET.to_vec()).unwrap(),
            expiry_secs: 3600,
        }
    }

    fn encode_api() -> String {
        encode(
            &Header::default(),
            &JwtClaims::api("1".to_string(), FUTURE_EXP),
            &EncodingKey::from_secret(SECRET),
        )
        .expect("encode api token")
    }

    fn encode_media() -> String {
        encode(
            &Header::default(),
            &JwtClaims::media("1".to_string(), FUTURE_EXP),
            &EncodingKey::from_secret(SECRET),
        )
        .expect("encode media token")
    }

    fn encode_ws() -> String {
        encode(
            &Header::default(),
            &JwtClaims::ws("1".to_string(), FUTURE_EXP),
            &EncodingKey::from_secret(SECRET),
        )
        .expect("encode ws ticket")
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("Authorization", format!("Bearer {token}").parse().unwrap());
        h
    }

    fn cookie(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, format!("{MEDIA_COOKIE}={token}").parse().unwrap());
        h
    }

    #[test]
    fn valid_api_bearer_is_authorized() {
        let auth = auth_state();
        assert!(media_request_user(&bearer(&encode_api()), &auth).is_some());
    }

    #[test]
    fn media_token_as_bearer_is_rejected() {
        // A media-scoped token presented as a bearer must NOT authorize.
        let auth = auth_state();
        assert!(media_request_user(&bearer(&encode_media()), &auth).is_none());
    }

    #[test]
    fn ws_ticket_as_bearer_is_rejected() {
        // A WebSocket ticket is scope-locked to the `/ws` upgrade. Presented as a
        // media bearer it must NOT authenticate — only a normal API token may
        // (`is_api()`, not merely `!is_media()`).
        let auth = auth_state();
        assert!(media_request_user(&bearer(&encode_ws()), &auth).is_none());
    }

    #[test]
    fn ws_ticket_in_cookie_is_rejected() {
        // The cookie path requires a media-scoped token; a ws ticket is rejected.
        let auth = auth_state();
        assert!(media_request_user(&cookie(&encode_ws()), &auth).is_none());
    }

    #[test]
    fn api_token_in_cookie_is_rejected() {
        // A full-access API token in the media cookie is rejected (cross-use guard).
        let auth = auth_state();
        assert!(media_request_user(&cookie(&encode_api()), &auth).is_none());
    }

    #[test]
    fn valid_media_cookie_is_authorized() {
        let auth = auth_state();
        assert!(media_request_user(&cookie(&encode_media()), &auth).is_some());
    }

    #[test]
    fn bearer_takes_precedence_over_cookie() {
        // Valid API bearer + valid media cookie present together. The bearer arm
        // is consulted first, so this is authorized via the bearer.
        let auth = auth_state();
        let mut h = bearer(&encode_api());
        h.insert(
            COOKIE,
            format!("{MEDIA_COOKIE}={}", encode_media())
                .parse()
                .unwrap(),
        );
        assert!(media_request_user(&h, &auth).is_some());
    }

    #[test]
    fn bearer_precedence_rejects_media_bearer_even_with_valid_cookie() {
        // Media token as bearer + valid media cookie: bearer wins and is the wrong
        // scope, so the whole request is rejected (cookie is never consulted).
        let auth = auth_state();
        let mut h = bearer(&encode_media());
        h.insert(
            COOKIE,
            format!("{MEDIA_COOKIE}={}", encode_media())
                .parse()
                .unwrap(),
        );
        assert!(media_request_user(&h, &auth).is_none());
    }

    #[test]
    fn returns_decoded_user_id() {
        // Authorized requests yield the token's `sub` (here "1") as the user id —
        // the value the route hands to the subscription guard.
        let auth = auth_state();
        assert_eq!(media_request_user(&bearer(&encode_api()), &auth), Some(1));
        assert_eq!(media_request_user(&cookie(&encode_media()), &auth), Some(1));
    }

    #[test]
    fn both_absent_is_rejected() {
        let auth = auth_state();
        assert!(media_request_user(&HeaderMap::new(), &auth).is_none());
    }

    #[test]
    fn wrong_secret_bearer_is_rejected() {
        // Token signed with a different secret fails signature verification.
        let token = encode(
            &Header::default(),
            &JwtClaims::api("1".to_string(), FUTURE_EXP),
            &EncodingKey::from_secret(b"a-different-secret"),
        )
        .unwrap();
        assert!(media_request_user(&bearer(&token), &auth_state()).is_none());
    }
}
