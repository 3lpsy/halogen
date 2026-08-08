use crate::routers::extractors::{AuthUserId, Id};
use axum::{Extension, Json};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::playback_delete::handle as playback_delete;
use crate::routers::ApiError;

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Id(playback_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    playback_delete(&dbc, user_id, playback_id)
        .await
        .map_err(|err| {
            warn!("Error deleting playback: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, authed_json, field_error, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_delete_playback_returns_ok() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        // Create a playback first
        let episode_id: i32 = payload
            .get("episode_id_1")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let store_data = serde_json::json!({
            "data": { "episode_id": episode_id, "cursor": 100, "completed": false },
            "params": null,
        });

        let response = router
            .clone()
            .oneshot(authed_json(
                "POST",
                "/api/v1/playbacks",
                &token,
                &store_data,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = json_body(response).await;
        let playback_id = json
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|id| id.as_i64())
            .expect("playback id");

        // Now delete it
        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/playbacks/{}", playback_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_delete_playback_with_invalid_jwt() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let fake_id = 999999;
        let token = generate_jwt_token(&fake_id.to_string());

        let response = router
            .clone()
            .oneshot(authed("DELETE", "/api/v1/playbacks/0", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_delete_playback_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("DELETE", "/api/v1/playbacks/abc", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = json_body(response).await;
        assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "parsing");
        assert_eq!(
            field_error(&json, "id"),
            "Invalid ID format: expected integer, got 'abc'"
        );
    }
}
