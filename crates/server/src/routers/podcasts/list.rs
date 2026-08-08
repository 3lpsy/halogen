use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PodcastData, PodcastInclude, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast::podcast_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<DefaultListParams<PodcastInclude>>,
) -> Result<Json<ResponseData<Vec<PodcastData>>>, ApiError> {
    let (podcasts, paginator) = podcast_list::handle(&dbc, user_id, &params)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_paginator(podcasts, paginator)))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_list_podcasts_returns_all() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/podcasts", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let podcasts = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        assert_eq!(podcasts.len(), 2, "should return 2 seeded podcasts");
    }

    #[tokio::test]
    async fn test_list_podcasts_with_invalid_jwt() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let fake_id = 999999;
        let token = generate_jwt_token(&fake_id.to_string());

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/podcasts", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_podcasts_invalid_page_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/podcasts?pagination[page]=-1&pagination[size]=10",
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
    async fn test_list_podcasts_invalid_size_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/podcasts?pagination[page]=1&pagination[size]=0",
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
    async fn test_list_podcasts_too_many_includes_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/podcasts?includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig&includes[]=PodcastConfig",
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

    #[tokio::test]
    async fn test_list_podcasts_with_valid_include() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/podcasts?includes[]=PodcastConfig",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let podcasts = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        assert_eq!(podcasts.len(), 2, "should return 2 seeded podcasts");
    }
}
