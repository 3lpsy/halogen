//! Media-only credentials use an HttpOnly `auth_media` cookie scoped to `/api/v1`, avoiding query-token leaks. Other
//! API routes ignore it. HTTPS/loopback use SameSite=None; Secure for cross-origin media; plain HTTP LAN uses
//! SameSite=Lax because browsers reject Secure there and None requires Secure.

use axum::http::{
    HeaderMap, HeaderValue,
    header::{COOKIE, HOST},
};

/// Cookie name carrying the media-only JWT.
pub const MEDIA_COOKIE: &str = "auth_media";

/// Path the cookie is scoped to — the API prefix, so all media routes
/// (`/episodes/{id}/audio`, `/episodes/{id}/art`, `/podcasts/{id}/art`)
/// receive it. Was `/api/v1/episodes` before podcast artwork existed.
const COOKIE_PATH: &str = "/api/v1";

/// Choose secure media-cookie attributes for TLS proxy requests (`X-Forwarded-Proto: https`) or loopback hosts, where
/// browsers also honor Secure over HTTP. See module docs for the LAN fallback.
pub fn secure_cookie_context(headers: &HeaderMap) -> bool {
    let forwarded_https = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("https"));
    forwarded_https
        || headers
            .get(HOST)
            .and_then(|v| v.to_str().ok())
            .is_some_and(host_is_loopback)
}

/// `localhost` / `127.0.0.1` / `[::1]`, with or without a port.
fn host_is_loopback(host: &str) -> bool {
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    bare.eq_ignore_ascii_case("localhost") || bare == "127.0.0.1" || bare == "::1"
}

/// The context-dependent tail: secure contexts get the cross-origin-capable
/// pair; plain-http gets `Lax` (never `None`-without-`Secure`, which browsers
/// reject, and never a `Secure` the browser would silently drop).
fn security_attrs(secure: bool) -> &'static str {
    if secure {
        "Secure; SameSite=None"
    } else {
        "SameSite=Lax"
    }
}

/// Build a `Set-Cookie` header value that stores `token` as the `auth_media`
/// cookie for `max_age_secs` seconds. `secure` per [`secure_cookie_context`].
pub fn set_media_cookie(token: &str, max_age_secs: u64, secure: bool) -> HeaderValue {
    // `token` is a base64url JWT (no `;`/whitespace), so this is always a valid
    // header value; fall back defensively rather than panicking.
    let attrs = security_attrs(secure);
    HeaderValue::from_str(&format!(
        "{MEDIA_COOKIE}={token}; HttpOnly; {attrs}; Path={COOKIE_PATH}; Max-Age={max_age_secs}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Build a `Set-Cookie` header value that immediately expires the cookie. Built
/// from the same `MEDIA_COOKIE`/`COOKIE_PATH` consts as [`set_media_cookie`] so a
/// rename can't leave the clear path pointing at a stale name/path.
pub fn clear_media_cookie(secure: bool) -> HeaderValue {
    let attrs = security_attrs(secure);
    HeaderValue::from_str(&format!(
        "{MEDIA_COOKIE}=; HttpOnly; {attrs}; Path={COOKIE_PATH}; Max-Age=0"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Read the `auth_media` value out of the request's `Cookie` header, if present.
pub fn read_media_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    // `Cookie: a=1; auth_media=xyz; b=2` — split on `;`, match our name.
    raw.split(';')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name.trim() == MEDIA_COOKIE).then(|| value.trim().to_string()))
}
