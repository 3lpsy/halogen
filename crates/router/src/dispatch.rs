use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use tower::ServiceExt;

const MAX_BODY: usize = 16 * 1024 * 1024;

pub use halogen_wire_meta::api::{ApiRequest, ApiResponse};

/// Execute a bounded application request without creating a network listener.
pub async fn dispatch(router: &Router, request: ApiRequest) -> Result<ApiResponse, String> {
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|_| "invalid method")?;
    if !matches!(
        method,
        Method::GET | Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        return Err("unsupported method".into());
    }
    if !request.path.starts_with("/api/v1/")
        || request.path.len() > 2048
        || request.path.contains(['?', '#', '\\'])
        || request.path.contains("..")
    {
        return Err("invalid API path".into());
    }
    let query = request.query.unwrap_or_default();
    if query.len() > 16 * 1024 || query.contains(['#', '\r', '\n']) {
        return Err("invalid query".into());
    }
    let transfer = matches!(
        request.path.as_str(),
        "/api/v1/admin/db/import" | "/api/v1/admin/db/export"
    );
    let limit = if transfer {
        halogen_routes::db_transfer::DB_IMPORT_MAX_BYTES
    } else {
        MAX_BODY
    };
    let body = request.body.unwrap_or_default();
    if body.len() > limit {
        return Err("request body exceeds limit".into());
    }
    let uri = if query.is_empty() {
        request.path
    } else {
        format!("{}?{query}", request.path)
    };
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .map_err(|_| "invalid request")?;
    let response = router
        .clone()
        .oneshot(request)
        .await
        .map_err(|_| "dispatch failed")?;
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), limit)
        .await
        .map_err(|_| "response exceeds limit")?
        .to_vec();
    Ok(ApiResponse { status, body })
}
