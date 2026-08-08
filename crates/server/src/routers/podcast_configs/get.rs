use axum::{Extension, Json};
use halogen_wire::{PodcastConfigData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast_config::podcast_config_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(config_id): Id,
) -> Result<Json<ResponseData<PodcastConfigData>>, ApiError> {
    // Read gate: only an owner of the podcast that references this config (or an
    // admin) may read it. A standalone config is owner-undecidable → 403 for a
    // non-admin, so reads can't leak another owner's config by guessing ids.
    guards::require_config_owner_or_admin(&dbc, actor, config_id).await?;
    let data = podcast_config_get::handle(&dbc, config_id).await?;
    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
mod tests {
    use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::authed;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_podcast_config_success() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        // The seeded config is standalone (no podcast references it), so config
        // ownership is undecidable; owners/admins read configs, so act as admin.
        let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
        let config_id = payload.get("podcast_config_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(admin_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/podcast-configs/{}", config_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_podcast_config_not_found() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let fake_id = i32::MAX.to_string();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/podcast-configs/{}", fake_id),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_podcast_config_invalid_id() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .clone()
            .oneshot(authed("GET", "/api/v1/podcast-configs/abc", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
