use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn get_config(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder().method("GET").uri("/api/v1/admin/config");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// No token → 401 (the route is behind the JWT layer).
#[tokio::test]
async fn config_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        get_config(&router, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

/// A non-admin authed user → 403.
#[tokio::test]
async fn config_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        get_config(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

/// Admin gets the reconciled config, and the secret fields are absent from
/// the JSON while a known non-secret field is present and correct.
#[tokio::test]
async fn config_admin_succeeds_and_omits_secrets() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

    let resp = get_config(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;

    let data = &json["data"];
    assert!(data.is_object(), "config payload present");
    // A representative non-secret field comes through.
    assert!(
        data["listen_port"].is_number(),
        "listen_port should be reported"
    );
    // Secrets must never be serialised.
    assert!(
        data.get("auth_token_secret").is_none(),
        "auth_token_secret must be omitted; got: {data}"
    );
    assert!(
        data.get("admin_password").is_none(),
        "admin_password must be omitted; got: {data}"
    );
}

/// Issue a `/config-overrides` request with an optional token + body.
async fn overrides_req(
    router: &axum::Router,
    method: &str,
    token: Option<&str>,
    body: Body,
) -> axum::response::Response {
    let mut b = Request::builder()
        .method(method)
        .uri("/api/v1/admin/config-overrides")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

/// No token → 401 (behind the JWT layer like GET /config).
#[tokio::test]
async fn overrides_get_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        overrides_req(&router, "GET", None, Body::empty())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

/// A non-admin authed user is forbidden on every verb.
#[tokio::test]
async fn overrides_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        overrides_req(&router, "GET", Some(&user), Body::empty())
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        // AdminUser gate runs before the Body extractor, so the body is moot.
        overrides_req(&router, "POST", Some(&user), Body::from(r#"{"data":{}}"#))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        overrides_req(&router, "DELETE", Some(&user), Body::empty())
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

/// Admin GET (no overrides file resolved in the test config) → 200 with an
/// empty object, the shape the editor prepopulates from.
#[tokio::test]
async fn overrides_admin_get_returns_object() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
    let resp = overrides_req(&router, "GET", Some(&admin), Body::empty()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(
        json["data"].is_object(),
        "overrides payload present; got: {json}"
    );
}
