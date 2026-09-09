use axum::http::{
    HeaderMap,
    header::{COOKIE, HOST},
};

use super::cookie::*;

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

#[test]
fn unreasonable_token_expiries_return_errors_without_panicking() {
    assert!(super::token::issue_tokens("secret", "1", u64::MAX, false).is_err());
    assert!(super::token::mint_ws_ticket("secret", "1", u64::MAX).is_err());
}
