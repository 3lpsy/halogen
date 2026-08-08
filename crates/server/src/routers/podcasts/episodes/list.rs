use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, FilterParams, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_list;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// `GET /podcasts/{id}/episodes` — the nested episode list.
///
/// This is a thin wrapper that injects the path `podcast_id` into the filter and
/// delegates to the **same** `episode_list::handle` used by `GET /episodes`, so
/// there is exactly one episode-listing implementation.
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Query(mut params): Query<DefaultListParams<EpisodeInclude>>,
) -> Result<Json<ResponseData<Vec<EpisodeData>>>, ApiError> {
    // Read gate: only subscribers (or owner/admin) may list a podcast's episodes;
    // an unsubscribed or non-existent podcast yields 404 (doesn't leak existence).
    guards::require_subscribed(&dbc, actor, podcast_id).await?;

    let mut filter = params.filter.take().unwrap_or_else(FilterParams::default);
    filter.podcast_id = Some(podcast_id);
    params.filter = Some(filter);

    let (episodes, paginator) = episode_list::handle(&dbc, actor.id, &params)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_paginator(episodes, paginator)))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// No bearer token → the auth middleware rejects with 401 before the handler runs.
    #[tokio::test]
    async fn test_list_podcast_episodes_requires_auth() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();

        let response = router
            .clone()
            .oneshot(unauthed(
                "GET",
                format!("/api/v1/podcasts/{}/episodes", podcast_id),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Existing podcast with auth → 200 and `data` is an array. The podcasts test
    /// helper seeds podcasts but no episodes, so the array is empty here.
    #[tokio::test]
    async fn test_list_podcast_episodes_success_returns_array() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/podcasts/{}/episodes", podcast_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let episodes = json
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data is array");
        // Helper seeds no episodes for the podcast.
        assert_eq!(episodes.len(), 0);
    }

    /// The nested list is read-gated by `require_subscribed`, so a podcast the
    /// caller isn't subscribed to (here: a non-existent id) yields 404 — not a
    /// 200 + empty array — and doesn't leak the podcast's (non-)existence.
    #[tokio::test]
    async fn test_list_podcast_episodes_nonexistent_podcast_returns_404() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/podcasts/123456/episodes", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Non-integer path id → the `Id` extractor rejects with 400 before the handler.
    #[tokio::test]
    async fn test_list_podcast_episodes_non_integer_id_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/podcasts/not-an-int/episodes",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
