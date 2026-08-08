use crate::routers::extractors::{AuthUserId, Body};
use axum::{Extension, Json};
use halogen_wire::{PlaylistData, PlaylistStoreData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_store::handle as playlist_store;
use crate::routers::ApiError;

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Body(store_data): Body<PlaylistStoreData>,
) -> Result<Json<ResponseData<PlaylistData>>, ApiError> {
    let data = playlist_store(&dbc, user_id, store_data)
        .await
        .map_err(|err| {
            warn!("Error creating playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed_json, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_store_playlist_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        // The fixture seeds no default playlist; the first one created must be the
        // queue (`is_default`), so create it as such.
        let store_body = serde_json::json!({
            "data": {
                "name": "New Playlist",
                "description": "Playlist description",
                "is_default": true
            }
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playlists",
                &token,
                &store_body,
            ))
            .await
            .unwrap();

        let status = response.status();
        let json = json_body(response).await;
        assert_eq!(status, StatusCode::OK);

        let pl = json.get("data").expect("data field");
        assert_eq!(pl.get("name").unwrap().as_str().unwrap(), "New Playlist");
        assert!(pl.get("is_default").unwrap().as_bool().unwrap());
    }

    /// A new playlist is appended to the user's manual order: `position = max + 1`.
    /// The fixture seeds two playlists at positions 0 and 1, so the next is 2.
    #[tokio::test]
    async fn test_store_playlist_assigns_next_position() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let store_body = serde_json::json!({
            "data": { "name": "Third", "is_default": true }
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playlists",
                &token,
                &store_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        let pl = json.get("data").expect("data field");
        assert_eq!(pl.get("position").unwrap().as_i64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_store_playlist_missing_body() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let store_body = serde_json::json!({});
        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playlists",
                &token,
                &store_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    #[tokio::test]
    async fn test_store_playlist_empty_name_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let store_body = serde_json::json!({
            "data": {
                "name": ""
            }
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playlists",
                &token,
                &store_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["name"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(field_error(&json, "name"), "Playlist name is required");
    }

    /// The queue is the default playlist and may not exist. While none exists, a
    /// non-default create is rejected (the first playlist must be the queue) with a
    /// 400 keyed on `is_default`. The fixture seeds no default, so this triggers.
    #[tokio::test]
    async fn test_store_non_default_rejected_when_no_default() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let store_body = serde_json::json!({ "data": { "name": "Not the queue" } });

        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playlists",
                &token,
                &store_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["is_default"].is_array(),
            "rejection should be keyed on is_default; body: {json}"
        );
    }
}
