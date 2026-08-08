use axum::{Extension, Json};
use halogen_wire::{PlaybackData, PlaybackListParams, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::playback_list::handle as playback_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<PlaybackListParams>,
) -> Result<Json<ResponseData<Vec<PlaybackData>>>, ApiError> {
    let (playback_data, paginator) =
        playback_list(&dbc, user_id, &params).await.map_err(|err| {
            warn!("Error fetching playbacks: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(playback_data, paginator)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_list_playbacks_returns_empty() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playbacks", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let playbacks = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        assert_eq!(playbacks.len(), 0, "should return empty array");
    }

    #[tokio::test]
    async fn test_list_playbacks_with_invalid_jwt() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let fake_id = 999999;
        let token = generate_jwt_token(&fake_id.to_string());

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playbacks", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_playbacks_invalid_episode_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/playbacks?episode_id=0", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(json.get("errors").is_some());
        assert!(json["errors"]["episode_id"].is_array());
    }

    #[tokio::test]
    async fn test_list_playbacks_invalid_page() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playbacks?pagination[page]=-1&pagination[size]=10",
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
    async fn test_list_playbacks_invalid_size() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playbacks?pagination[page]=1&pagination[size]=0",
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
}
