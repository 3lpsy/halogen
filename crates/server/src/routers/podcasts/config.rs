//! `POST` / `DELETE /podcasts/{id}/config` — manage a podcast's download/poll
//! override config and its linkage in one atomic step.
//!
//! These nested routes are the ONLY way to create or delete a config row: there is
//! no standalone create/delete endpoint, only `GET`/`PUT /podcast-configs/{id}` for
//! reading + editing an existing one. Here the create links and the delete unlinks,
//! transactionally, keeping `podcast.podcast_config_id` consistent (see the
//! handlers). Editing an existing config goes through `PUT /podcast-configs/{id}`.

use axum::{Extension, Json};
use halogen_wire::{PodcastConfigData, PodcastConfigStoreData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast_config::{remove_for_podcast, store_for_podcast};
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

/// `POST /podcasts/{id}/config` — create + link a config for the podcast.
pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Body(data): Body<PodcastConfigStoreData>,
) -> Result<Json<ResponseData<PodcastConfigData>>, ApiError> {
    // Only the podcast's owner (or an admin) may set its config.
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    let config = store_for_podcast::handle(&dbc, podcast_id, data).await?;
    Ok(Json(ResponseData::from_data(config)))
}

/// `DELETE /podcasts/{id}/config` — unlink + delete the podcast's config.
pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    remove_for_podcast::handle(&dbc, podcast_id).await?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, authed_json, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// Create a config for a podcast that has none → 200, and the returned config
    /// carries the submitted override.
    #[tokio::test]
    async fn test_store_config_for_podcast_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "poll_interval_seconds": 300 } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                format!("/api/v1/podcasts/{}/config", podcast_id),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert_eq!(json["data"]["poll_interval_seconds"], 300);
    }

    /// Creating a second config for a podcast that already has one → rejected.
    #[tokio::test]
    async fn test_store_config_for_podcast_already_exists() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "max_episodes": 10 } });
        let mk = || {
            authed_json(
                "POST",
                format!("/api/v1/podcasts/{}/config", podcast_id),
                &token,
                &body,
            )
        };

        let first = router.clone().oneshot(mk()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        let second = router.clone().oneshot(mk()).await.unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    /// Create config for a missing podcast → 404.
    #[tokio::test]
    async fn test_store_config_for_missing_podcast_404() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "max_episodes": 10 } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/podcasts/123456/config",
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Out-of-range override → 400 (validated by the `Body` extractor).
    #[tokio::test]
    async fn test_store_config_for_podcast_invalid_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({ "data": { "poll_interval_seconds": 86401 } });
        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                format!("/api/v1/podcasts/{}/config", podcast_id),
                &token,
                &body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Remove a config: create one, then DELETE → 200, and a second DELETE is an
    /// idempotent no-op success.
    #[tokio::test]
    async fn test_remove_config_for_podcast() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let create_body = serde_json::json!({ "data": { "max_episodes": 10 } });
        let create = router
            .clone()
            .oneshot(authed_json(
                "POST",
                format!("/api/v1/podcasts/{}/config", podcast_id),
                &token,
                &create_body,
            ))
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::OK);

        let mk_delete = || {
            authed(
                "DELETE",
                format!("/api/v1/podcasts/{}/config", podcast_id),
                &token,
            )
        };

        let first = router.clone().oneshot(mk_delete()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        let second = router.clone().oneshot(mk_delete()).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);
    }
}
