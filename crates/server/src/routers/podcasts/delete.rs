use axum::{Extension, Json};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_delete;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_podcast_owner_or_admin(&dbc, actor, podcast_id).await?;

    // The handler confirms existence (404 if gone) and cascades the delete; it owns
    // the title for logging, so the router no longer pre-fetches the podcast.
    podcast_delete::handle(&dbc, podcast_id)
        .await
        .map_err(|err| {
            warn!("Error deleting podcast: {:?}", err);
            ApiError(err)
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_delete_podcast_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert!(json["data"].is_null());

        // Verify the podcast is actually deleted.
        let verify = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/podcasts/{podcast_id}"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(verify.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_podcast_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let fake_id = i32::MAX.to_string();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "DELETE",
                format!("/api/v1/podcasts/{fake_id}"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_podcast_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("DELETE", "/api/v1/podcasts/abc", &token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
