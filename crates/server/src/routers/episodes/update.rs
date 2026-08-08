use axum::{Extension, Json};
use halogen_wire::{EpisodeData, EpisodeUpdateData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_update;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    id: Id,
    Body(update_data): Body<EpisodeUpdateData>,
) -> Result<Json<ResponseData<EpisodeData>>, ApiError> {
    // Write gate: only the episode's podcast owner (or an admin) may edit it.
    guards::require_episode_writer(&dbc, actor, id.0).await?;

    let ep_data = episode_update::handle(&dbc, actor.id, id.0, &update_data)
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
    async fn test_update_episode_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let episode_id = payload.get("episode_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let update_body = serde_json::json!({
            "data": {
                "title": "Updated Episode Title"
            }
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", episode_id),
                &token,
                &update_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let ep_data = json.get("data").expect("data field");
        assert_eq!(
            ep_data.get("title").unwrap().as_str().unwrap(),
            "Updated Episode Title"
        );
    }

    #[tokio::test]
    async fn test_update_episode_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let fake_id = i32::MAX.to_string();

        let update_body = serde_json::json!({
            "data": {
                "title": "New Title"
            }
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", fake_id),
                &token,
                &update_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let json = json_body(response).await;
        assert_eq!(field_error(&json, "id"), "Episode not found");
    }

    #[tokio::test]
    async fn test_update_episode_missing_body() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let episode_id = payload.get("episode_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({});
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", episode_id),
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
    async fn test_update_episode_empty_title_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({
            "data": {
                "title": ""
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", episode_id),
                &token,
                &body,
            ))
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
    async fn test_update_episode_invalid_content_url_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({
            "data": {
                "content_url": "not-a-valid-url"
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", episode_id),
                &token,
                &body,
            ))
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
    async fn test_update_episode_title_too_long_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({
            "data": {
                "title": "a".repeat(257)
            }
        });
        let response = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/episodes/{}", episode_id),
                &token,
                &body,
            ))
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
}
