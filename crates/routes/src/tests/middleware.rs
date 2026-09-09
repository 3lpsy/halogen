use crate::routers::middleware::JwtClaims;
use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use jsonwebtoken::{EncodingKey, Header, encode};
use tower::ServiceExt;

/// The same secret `build_test_router` configures via the test `Config`.
const TEST_SECRET: &[u8] = b"test-secret-key-for-jwt-signing";

/// Encode a JWT against an arbitrary secret with an arbitrary expiry.
///
/// Mirrors `generate_jwt_token` but lets each test control the secret and
/// `exp` so we can exercise the wrong-secret and expired paths.
fn encode_token(sub: &str, exp: usize, secret: &[u8]) -> String {
    let claims = JwtClaims::api(sub.to_string(), exp);
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("encode token")
}

/// Fire a `GET /playbacks` (a protected route) and return the status. When
/// `auth` is `Some`, it is sent verbatim as the `Authorization` header.
async fn request_protected(auth: Option<&str>) -> StatusCode {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let mut builder = Request::builder().method("GET").uri("/api/v1/playbacks");
    if let Some(value) = auth {
        builder = builder.header("Authorization", value);
    }
    let request = builder.body(Body::empty()).unwrap();

    router.oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn test_missing_authorization_header_is_401() {
    // `extract_bearer` returns None → "Missing or malformed Authorization header".
    assert_eq!(request_protected(None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_header_without_bearer_prefix_is_401() {
    // No `Bearer ` prefix → `strip_prefix` returns None → 401.
    assert_eq!(
        request_protected(Some("token-without-prefix")).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn test_bearer_with_garbage_token_is_401() {
    // Present prefix but undecodable token → `decode` fails → 401.
    assert_eq!(
        request_protected(Some("Bearer not-a-real-jwt")).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn test_expired_token_is_401() {
    // Right secret, but `exp` in the past → `validate_exp` rejects it → 401.
    let past_exp = 1_000_000_000usize; // 2001-09-09, well in the past.
    let token = encode_token("1", past_exp, TEST_SECRET);
    assert_eq!(
        request_protected(Some(&format!("Bearer {token}"))).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn test_wrong_secret_token_is_401() {
    // Valid shape and future expiry, but signed with a different secret →
    // signature verification fails → 401.
    let future_exp = 4_000_000_000usize; // 2096, far future.
    let token = encode_token("1", future_exp, b"the-wrong-secret");
    assert_eq!(
        request_protected(Some(&format!("Bearer {token}"))).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn test_valid_token_for_nonexistent_user_is_401() {
    // Token decodes fine, but the `sub` user is absent from the DB. The
    // lookup's `ok_or(StatusCode::UNAUTHORIZED)` turns the missing row into
    // a 401 (not a 500/404).
    let token = generate_jwt_token("999999");
    assert_eq!(
        request_protected(Some(&format!("Bearer {token}"))).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn test_media_scoped_token_rejected_on_api_route() {
    // A correctly-signed, unexpired token whose `sub` is a real user but
    // carries `scope=media` must NOT authenticate an API route — the media
    // credential is for the audio endpoint only.
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let future_exp = 4_000_000_000usize;
    let media = encode(
        &Header::default(),
        &JwtClaims::media(user_id.to_string(), future_exp),
        &EncodingKey::from_secret(TEST_SECRET),
    )
    .expect("encode media token");

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/playbacks")
                .header("Authorization", format!("Bearer {media}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_valid_token_for_seeded_user_is_200() {
    // Positive control: a correctly-signed, unexpired token whose `sub`
    // matches a seeded user passes the middleware and reaches the handler.
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/playbacks")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
