use axum::{Json, http::HeaderMap, http::header::SET_COOKIE};
use halogen_wire::ResponseData;
use tracing::info;

use super::cookie::{clear_media_cookie, secure_cookie_context};

pub async fn logout(request_headers: HeaderMap) -> (HeaderMap, Json<ResponseData<()>>) {
    info!("User logged out (stateless)");
    // Expire the media cookie so a shared browser can't keep streaming. Attrs
    // match the set path's context so the overwrite lands on the same cookie.
    let secure = secure_cookie_context(&request_headers);
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, clear_media_cookie(secure));
    (headers, Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use axum::http::{StatusCode, header};
    use tower::ServiceExt;

    use super::super::tests::{build_test_router, setup_test_db};
    use crate::tests::harness::{json_body, unauthed};

    #[tokio::test]
    async fn test_logout_returns_ok() {
        let (_root, dbc, _meta) = setup_test_db().await;
        let router = build_test_router(dbc);

        let response = router
            .clone()
            .oneshot(unauthed("POST", "/api/v1/auth/logout"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Logout expires the media cookie.
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("logout must clear the cookie")
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("auth_media=;"), "cookie value cleared");
        assert!(cookie.contains("Max-Age=0"), "cookie expires immediately");

        let json = json_body(response).await;
        assert!(
            json.get("data").is_some(),
            "logout should return data field"
        );
    }
}
