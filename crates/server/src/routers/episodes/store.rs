use axum::{Extension, Json};
use halogen_wire::{EpisodeData, EpisodeStoreData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_store;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body};
use crate::routers::guards;

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Body(store_data): Body<EpisodeStoreData>,
) -> Result<Json<ResponseData<EpisodeData>>, ApiError> {
    // Create gate: only the parent podcast's owner (or an admin) may add episodes.
    guards::require_podcast_owner_or_admin(&dbc, actor, store_data.podcast_id).await?;

    let ep_data = episode_store::handle(&dbc, store_data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(ep_data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed_json, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_store_episode_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let store_body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "New Episode",
                "description": "Episode description",
                "content_url": "https://example.com/episode.mp3",
                "art_url": null
            }
        });

        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &store_body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let ep = json.get("data").expect("data field");
        assert_eq!(ep.get("title").unwrap().as_str().unwrap(), "New Episode");
        assert_eq!(
            ep.get("content_url").unwrap().as_str().unwrap(),
            "https://example.com/episode.mp3"
        );
    }

    #[tokio::test]
    async fn test_store_episode_missing_body() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({});
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    #[tokio::test]
    async fn test_store_episode_empty_title_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "",
                "description": "d",
                "content_url": "https://example.com/ep.mp3"
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["title"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(
            field_error(&json, "title"),
            "Title must be between 1 and 256 characters long"
        );
    }

    #[tokio::test]
    async fn test_store_episode_invalid_content_url_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "New Episode",
                "description": "d",
                "content_url": "not-a-valid-url"
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["content_url"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(
            field_error(&json, "content_url"),
            "Content URL must be a valid URL"
        );
    }

    #[tokio::test]
    async fn test_store_episode_title_too_long_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "a".repeat(257),
                "description": "d",
                "content_url": "https://example.com/ep.mp3"
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["title"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(
            field_error(&json, "title"),
            "Title must be between 1 and 256 characters long"
        );
    }

    #[tokio::test]
    async fn test_store_episode_invalid_art_url_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "New Episode",
                "description": "d",
                "content_url": "https://example.com/ep.mp3",
                "art_url": "not-a-url"
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["art_url"].is_array(),
            "validation errors should be keyed by field name"
        );
        assert_eq!(field_error(&json, "art_url"), "Art URL must be a valid URL");
    }

    #[tokio::test]
    async fn test_store_episode_description_too_long_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let podcast_id: i32 = payload
            .get("podcast_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let body = serde_json::json!({
            "data": {
                "podcast_id": podcast_id,
                "title": "New Episode",
                "content_url": "https://example.com/ep.mp3",
                "description": "a".repeat(65537)
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json("POST", "/api/v1/episodes", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert!(
            json["errors"]["description"].is_array(),
            "validation errors should be keyed by field name"
        );
    }
}
