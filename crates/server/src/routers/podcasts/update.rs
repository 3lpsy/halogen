use axum::{Extension, Json};
use halogen_wire::{PodcastData, PodcastUpdateData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_update;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Body(update_data): Body<PodcastUpdateData>,
) -> Result<Json<ResponseData<PodcastData>>, ApiError> {
    // After body validation, before mutating: owner-or-admin only.
    guards::require_podcast_owner_or_admin(&dbc, actor, podcast_id).await?;
    let podcast_data = podcast_update::handle(&dbc, podcast_id, update_data)
        .await
        .map_err(|err| {
            warn!("Error updating podcast: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(podcast_data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed_json, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_update_podcast_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "title": "Updated Podcast Title" } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert_eq!(
            json["data"]["title"].as_str().unwrap(),
            "Updated Podcast Title"
        );
    }

    #[tokio::test]
    async fn test_update_podcast_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let fake_id = i32::MAX.to_string();

        let body = serde_json::json!({ "data": { "title": "New Title" } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/podcasts/{fake_id}"),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "id"), "Podcast not found");
    }

    #[tokio::test]
    async fn test_update_podcast_missing_body() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({});
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    #[tokio::test]
    async fn test_update_podcast_title_too_long() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "title": "a".repeat(257) } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(
            field_error(&json, "title"),
            "Title must be between 1 and 256 characters long"
        );
    }

    #[tokio::test]
    async fn test_update_podcast_invalid_feed_url() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "feed_url": "not-a-valid-url" } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(
            field_error(&json, "feed_url"),
            "Feed URL must be a valid URL"
        );
    }
}
