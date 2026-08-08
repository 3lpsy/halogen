use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PlaylistData, PlaylistInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_list::handle as playlist_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<DefaultListParams<PlaylistInclude>>,
) -> Result<Json<ResponseData<Vec<PlaylistData>>>, ApiError> {
    let (playlist_data, paginator) =
        playlist_list(&dbc, user_id, &params).await.map_err(|err| {
            warn!("Error fetching playlists: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(playlist_data, paginator)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_list_playlists_returns_all() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playlists", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let playlists = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        assert_eq!(playlists.len(), 2, "should return 2 seeded playlists");
    }

    #[tokio::test]
    async fn test_list_playlists_with_invalid_jwt() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let fake_id = 999999;
        let token = generate_jwt_token(&fake_id.to_string());

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playlists", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_playlists_invalid_page_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playlists?pagination[page]=-1&pagination[size]=10",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["pagination.page"].is_array(),
            "validation errors should be keyed by field name"
        );
    }

    #[tokio::test]
    async fn test_list_playlists_invalid_size_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playlists?pagination[page]=1&pagination[size]=0",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["pagination.size"].is_array(),
            "validation errors should be keyed by field name"
        );
    }

    #[tokio::test]
    async fn test_list_playlists_too_many_includes_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playlists?includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes&includes[]=Episodes",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert!(
            json.get("errors").is_some(),
            "error response must have errors envelope"
        );
        assert!(
            json["errors"]["includes"].is_array(),
            "validation errors should be keyed by field name"
        );
    }

    /// `filter[search]` narrows by name (LIKE). Seeded names are "Test Playlist 1"
    /// / "Test Playlist 2"; searching "1" matches only the first.
    #[tokio::test]
    async fn test_list_playlists_search_filters_by_name() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playlists?filter[search]=1", &token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        let data = json.get("data").unwrap().as_array().unwrap();
        assert_eq!(data.len(), 1, "only the matching playlist");
        assert_eq!(
            data[0].get("id").unwrap().to_string(),
            playlist_id_1.to_string()
        );
    }

    #[tokio::test]
    async fn test_list_playlists_with_valid_include() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playlists?includes[]=Episodes",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let playlists = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        assert_eq!(playlists.len(), 2, "should return 2 seeded playlists");
    }
}
