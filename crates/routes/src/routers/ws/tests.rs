use super::*;
use crate::routers::middleware::JwtClaims;
use jsonwebtoken::{EncodingKey, Header, encode};

const SECRET: &[u8] = b"ws-auth-test-secret";
const FUTURE_EXP: usize = 4_000_000_000;

fn auth() -> AuthConfig {
    AuthConfig {
        secret: String::from_utf8(SECRET.to_vec()).unwrap(),
        expiry_secs: 3600,
    }
}

fn encode_with(claims: JwtClaims) -> String {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(SECRET),
    )
    .expect("encode")
}

#[test]
fn valid_ws_ticket_authorizes() {
    let ticket = encode_with(JwtClaims::ws("7".to_string(), FUTURE_EXP));
    assert_eq!(ws_ticket_user(Some(&ticket), &auth()), Some(7));
}

#[test]
fn api_token_rejected_as_ticket() {
    // A full-access API token must not work as a WS ticket (scope-locked).
    let api = encode_with(JwtClaims::api("7".to_string(), FUTURE_EXP));
    assert!(ws_ticket_user(Some(&api), &auth()).is_none());
}

#[test]
fn media_token_rejected_as_ticket() {
    let media = encode_with(JwtClaims::media("7".to_string(), FUTURE_EXP));
    assert!(ws_ticket_user(Some(&media), &auth()).is_none());
}

#[test]
fn expired_ticket_rejected() {
    let past = 1_000_000_000usize; // 2001 — well past.
    let ticket = encode_with(JwtClaims::ws("7".to_string(), past));
    assert!(ws_ticket_user(Some(&ticket), &auth()).is_none());
}

#[test]
fn wrong_secret_ticket_rejected() {
    let ticket = encode(
        &Header::default(),
        &JwtClaims::ws("7".to_string(), FUTURE_EXP),
        &EncodingKey::from_secret(b"a-different-secret"),
    )
    .unwrap();
    assert!(ws_ticket_user(Some(&ticket), &auth()).is_none());
}

#[test]
fn missing_ticket_rejected() {
    assert!(ws_ticket_user(None, &auth()).is_none());
}

#[test]
fn ping_gets_pong_with_echoed_ts() {
    let reply = reply_for(r#"{"t":"ping","ts":1234}"#).expect("pong");
    assert_eq!(reply, r#"{"t":"pong","ts":1234}"#);
}

#[test]
fn non_ping_text_ignored() {
    assert!(reply_for(r#"{"t":"hello"}"#).is_none());
    assert!(reply_for("not json").is_none());
}
