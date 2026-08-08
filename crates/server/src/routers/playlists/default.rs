use axum::{Extension, Json};
use halogen_wire::{DefaultPlaylistData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_default::handle as playlist_default;
use crate::routers::ApiError;
use crate::routers::extractors::AuthUserId;

/// `GET /playlists/default` — the user's queue (default playlist) or `null`.
pub async fn default(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
) -> Result<Json<ResponseData<DefaultPlaylistData>>, ApiError> {
    let data = playlist_default(&dbc, user_id).await.map_err(|err| {
        warn!("Error fetching default playlist: {:?}", err);
        err
    })?;

    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, authed_json, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// With a seeded default playlist, `/playlists/default` returns it.
    #[tokio::test]
    async fn returns_the_default_playlist() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc.clone());
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        // Promote playlist_id_1 to default first (the fixture seeds none as default).
        let playlist_id_1 = payload.get("playlist_id_1").unwrap().as_str().unwrap();
        let promote_body = serde_json::json!({"data":{"is_default":true}});
        let promote = router
            .clone()
            .oneshot(authed_json(
                "PUT",
                format!("/api/v1/playlists/{playlist_id_1}"),
                &token,
                &promote_body,
            ))
            .await
            .unwrap();
        assert_eq!(promote.status(), StatusCode::OK);

        let response = router
            .oneshot(authed("GET", "/api/v1/playlists/default", &token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        let pl = json.get("data").unwrap().get("playlist").unwrap();
        assert_eq!(
            pl.get("id").unwrap().as_i64().unwrap(),
            playlist_id_1.parse::<i64>().unwrap(),
            "default endpoint returns the promoted playlist; body: {json}"
        );
    }

    /// With no default seeded, `/playlists/default` returns `playlist: null` (200,
    /// not a 404) — "no queue" is a state, not an error.
    #[tokio::test]
    async fn returns_null_when_no_default() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(authed("GET", "/api/v1/playlists/default", &token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert!(
            json.get("data").unwrap().get("playlist").unwrap().is_null(),
            "no default → playlist null; body: {json}"
        );
    }
}
