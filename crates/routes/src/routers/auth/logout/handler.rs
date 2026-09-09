use axum::{Json, http::HeaderMap, http::header::SET_COOKIE};
use halogen_wire::ResponseData;
use tracing::info;

use super::super::cookie::{clear_media_cookie, secure_cookie_context};

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
#[path = "tests.rs"]
mod tests;
