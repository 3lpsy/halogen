use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PlaylistData, PlaylistInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::episode::episode_playlists::handle as episode_playlists;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// `GET /episodes/{id}/playlists` — the caller's playlists that contain this
/// episode (pre-selection for the "add to playlist" picker).
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(episode_id): Id,
    Query(params): Query<DefaultListParams<PlaylistInclude>>,
) -> Result<Json<ResponseData<Vec<PlaylistData>>>, ApiError> {
    // Read-gate on subscription like the other episode-id routes (404 hides existence).
    guards::require_episode_subscribed(&dbc, actor, episode_id).await?;
    let (data, paginator) = episode_playlists(&dbc, actor.id, episode_id, &params)
        .await
        .map_err(|err| {
            warn!("Error fetching episode playlists: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(data, paginator)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// episode_1 is seeded into playlist_1 → membership lists playlist_1.
    #[tokio::test]
    async fn test_episode_playlists_lists_members() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let episode_id_1 = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{}/playlists", episode_id_1),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        let data = json.get("data").unwrap().as_array().unwrap();
        let ids: Vec<String> = data
            .iter()
            .map(|p| p.get("id").unwrap().to_string())
            .collect();
        assert_eq!(ids, vec![playlist_id_1.to_string()]);
    }

    /// episode_2 is in no playlist → empty membership.
    #[tokio::test]
    async fn test_episode_playlists_empty_when_no_membership() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let episode_id_2 = payload.get("episode_id_2").unwrap().as_str().unwrap();

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{}/playlists", episode_id_2),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert!(json.get("data").unwrap().as_array().unwrap().is_empty());
    }

    /// Membership without a bearer token → 401.
    #[tokio::test]
    async fn test_episode_playlists_requires_auth() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let episode_id_1 = payload.get("episode_id_1").unwrap().as_str().unwrap();

        let response = router
            .clone()
            .oneshot(unauthed(
                "GET",
                format!("/api/v1/episodes/{}/playlists", episode_id_1),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
