use crate::routers::extractors::{AuthUserId, Id, Query};
use axum::{Extension, Json};
use halogen_wire::{PlaylistData, PlaylistShowParams, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_get::handle as playlist_get;
use crate::routers::ApiError;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Id(playlist_id): Id,
    Query(params): Query<PlaylistShowParams>,
) -> Result<Json<ResponseData<PlaylistData>>, ApiError> {
    let playlist_data = playlist_get(&dbc, user_id, playlist_id, params.includes.as_ref())
        .await
        .map_err(|err| {
            warn!("Error fetching playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(playlist_data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_playlist_returns_playlist() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let playlist_id = payload.get("playlist_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/playlists/{}", playlist_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let pl = json.get("data").expect("data");
        assert_eq!(pl.get("name").unwrap().as_str().unwrap(), "Test Playlist 1");
    }

    #[tokio::test]
    async fn test_get_playlist_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let fake_id = i32::MAX.to_string();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/playlists/{}", fake_id),
                &token,
            ))
            .await
            .unwrap();

        let status = response.status();
        let json = json_body(response).await;
        println!("JSON: {}", serde_json::to_string_pretty(&json).unwrap());

        assert_eq!(status, StatusCode::NOT_FOUND);

        let errors_array = json
            .get("errors")
            .unwrap()
            .get("id")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            errors_array[0].get("code").unwrap().as_str().unwrap(),
            "exists"
        );
        assert_eq!(field_error(&json, "id"), "Playlist not found");
    }

    #[tokio::test]
    async fn test_get_playlist_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playlists/abc", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        let errors_array = json
            .get("errors")
            .unwrap()
            .get("id")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            errors_array[0].get("code").unwrap().as_str().unwrap(),
            "parsing"
        );
        assert_eq!(
            field_error(&json, "id"),
            "Invalid ID format: expected integer, got 'abc'"
        );
    }
}
