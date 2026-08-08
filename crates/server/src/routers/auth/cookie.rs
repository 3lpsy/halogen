//! `auth_media` cookie helpers.
//!
//! Media serving uses browser-issued requests (`<audio src>` streams, `<img src>`
//! artwork) which can't carry an `Authorization` header — but they *do* send
//! cookies. So the media credential travels as an `HttpOnly` cookie rather than
//! a URL query token (no logs/referrer leak), scoped to the `/api/v1` API prefix
//! (it must reach both `/episodes/{id}/{audio,art}` and `/podcasts/{id}/art`;
//! every other API route ignores cookies, and the token is media-scoped — it
//! can't be used as an API credential).
//!
//! The `Secure`/`SameSite` attributes depend on the REQUEST's context
//! ([`secure_cookie_context`]): over https (TLS proxy) or loopback,
//! `SameSite=None; Secure` — required for the cross-origin dev setup (UI on
//! one port, API on another; `localhost` is a secure context, so `Secure` is
//! honoured over `http://localhost`). On a plain-http non-loopback deployment
//! (a LAN install), `Secure` would make the browser silently DROP the cookie —
//! and `SameSite=None` is spec-invalid without `Secure` — so there the cookie
//! is `SameSite=Lax` instead, which same-origin media requests always send.
//!
//! No cookie crate is pulled in — the header is a single well-defined line, so
//! we build and parse it directly.

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

/// Whether the requesting browser is in a *secure context* for cookie
/// purposes: the request came through a TLS-terminating proxy
/// (`X-Forwarded-Proto: https` — this server only ever serves plain HTTP
/// itself) or from a loopback host (browsers treat `localhost` as trustworthy
/// and honour `Secure` cookies over plain http — the cross-origin dev setup
/// relies on that). Decides the media cookie's `Secure`/`SameSite` attributes
/// (see the module docs).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_has_expected_attributes() {
        let v = set_media_cookie("abc.def.ghi", 3600, true);
        let s = v.to_str().unwrap();
        assert!(s.starts_with("auth_media=abc.def.ghi;"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("Secure"));
        assert!(s.contains("SameSite=None"));
        assert!(s.contains("Path=/api/v1;"));
        assert!(s.contains("Max-Age=3600"));
    }

    #[test]
    fn plain_http_cookie_is_lax_and_not_secure() {
        // A `Secure` cookie over plain http is silently dropped by the browser
        // (all media auth dead), and `SameSite=None` without `Secure` is
        // rejected outright — the non-secure context must get Lax instead.
        let s = set_media_cookie("abc", 3600, false)
            .to_str()
            .unwrap()
            .to_string();
        assert!(!s.contains("Secure"));
        assert!(s.contains("SameSite=Lax"));
        assert!(s.contains("HttpOnly"));
        let s = clear_media_cookie(false).to_str().unwrap().to_string();
        assert!(!s.contains("Secure"));
        assert!(s.contains("SameSite=Lax"));
    }

    #[test]
    fn clear_cookie_expires_immediately() {
        let s = clear_media_cookie(true).to_str().unwrap().to_string();
        assert!(s.contains("Max-Age=0"));
        assert!(s.starts_with("auth_media=;"));
    }

    #[test]
    fn secure_context_from_proxy_or_loopback() {
        let mut h = HeaderMap::new();
        assert!(!secure_cookie_context(&h), "no signals → not secure");
        h.insert(HOST, "pods.lan:8080".parse().unwrap());
        assert!(!secure_cookie_context(&h), "plain-http LAN host");
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(secure_cookie_context(&h), "TLS proxy");
        h.remove("x-forwarded-proto");
        h.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!secure_cookie_context(&h), "explicit plain-http proxy");
        for host in [
            "localhost",
            "localhost:8000",
            "127.0.0.1:3000",
            "[::1]:8000",
        ] {
            let mut h = HeaderMap::new();
            h.insert(HOST, host.parse().unwrap());
            assert!(secure_cookie_context(&h), "loopback {host} is secure");
        }
    }

    #[test]
    fn reads_value_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "foo=1; auth_media=tok123; bar=2".parse().unwrap());
        assert_eq!(read_media_cookie(&headers).as_deref(), Some("tok123"));
    }

    #[test]
    fn missing_cookie_is_none() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "foo=1; bar=2".parse().unwrap());
        assert_eq!(read_media_cookie(&headers), None);
        assert_eq!(read_media_cookie(&HeaderMap::new()), None);
    }
}
