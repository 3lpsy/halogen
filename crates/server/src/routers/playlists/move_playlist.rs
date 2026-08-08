use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::{PlaylistMoveData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_move::handle_move;
use crate::routers::ApiError;

/// `POST /playlists/{id}/move` — move a playlist within its owner's manual
/// order. Owner-or-admin only; the handler scopes to the playlist's owner.
pub async fn move_playlist(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(data): Body<PlaylistMoveData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    handle_move(&dbc, playlist_id, data.to)
        .await
        .map_err(|err| {
            warn!("Error moving playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, authed_json, json, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// Move playlist 2 (seeded at position 1) to the front; the position-ordered
    /// list then returns it first.
    #[tokio::test]
    async fn test_move_playlist_reorders() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();
        let playlist_id_2 = payload.get("playlist_id_2").unwrap().as_str().unwrap();

        let move_body = serde_json::json!({"data":{"to":0}});
        let mv = router
            .clone()
            .oneshot(authed_json(
                "POST",
                format!("/api/v1/playlists/{}/move", playlist_id_2),
                &token,
                &move_body,
            ))
            .await
            .unwrap();
        assert_eq!(mv.status(), StatusCode::OK);

        let list = router
            .clone()
            .oneshot(authed(
                "GET",
                "/api/v1/playlists?order[order_by]=position&order[direction]=Asc",
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let json = json_body(list).await;
        let data = json.get("data").unwrap().as_array().unwrap();
        let ids: Vec<String> = data
            .iter()
            .map(|p| p.get("id").unwrap().to_string())
            .collect();
        assert_eq!(
            ids.first().unwrap(),
            playlist_id_2,
            "moved playlist is first"
        );
        assert_eq!(ids.get(1).unwrap(), playlist_id_1, "other playlist follows");
    }

    /// Move without a bearer token → 401.
    #[tokio::test]
    async fn test_move_playlist_requires_auth() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();

        let move_body = serde_json::json!({"data":{"to":0}});
        let response = router
            .clone()
            .oneshot(json(
                "POST",
                format!("/api/v1/playlists/{}/move", playlist_id_1),
                &move_body,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
