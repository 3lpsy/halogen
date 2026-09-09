use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> axum::response::Response {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn start_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        send(&router, "POST", "/api/v1/admin/poll-job", None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn start_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        send(&router, "POST", "/api/v1/admin/poll-job", Some(&user))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_starts_job_and_lists_it() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

    let resp = send(&router, "POST", "/api/v1/admin/poll-job", Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let job_id = json["data"]["job_id"].as_i64().expect("job_id present");
    assert!(job_id >= 1);

    // The job shows up in the history list.
    let resp = send(&router, "GET", "/api/v1/admin/poll-jobs", Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let jobs = json["data"].as_array().expect("array");
    assert!(jobs.iter().any(|j| j["id"].as_i64() == Some(job_id)));
}

#[tokio::test]
async fn get_unknown_job_is_404() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
    assert_eq!(
        send(&router, "GET", "/api/v1/admin/poll-job/99999", Some(&admin))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}
