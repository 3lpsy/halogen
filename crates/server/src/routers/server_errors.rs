//! Admin server-errors read (`GET /admin/server-errors`): the persisted failure
//! histories — podcast RSS sync failures and episode media-download failures —
//! newest first, with entity titles resolved for display.

use std::collections::HashMap;

use axum::{Json, extract::Extension};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use halogen_orm::{episode, episode_download_error, podcast, podcast_sync_error};
use halogen_utils::constants::VALIDATION_PANIC_CODE;
use halogen_wire::{
    EpisodeDownloadErrorData, PodcastSyncErrorData, ResponseData, ServerErrorsData,
};

use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;

/// Cap per error kind. The tables are already pruned per entity at write time;
/// this bounds the response for a library with many failing entities.
const MAX_ROWS: u64 = 200;

fn db_err(e: sea_orm::DbErr) -> ApiError {
    ApiError::new("server_errors", VALIDATION_PANIC_CODE, e.to_string())
}

/// GET /admin/server-errors — both failure histories in one read. **Admin only.**
#[axum::debug_handler]
pub async fn get(
    _admin: AdminUser,
    Extension(dbc): Extension<DatabaseConnection>,
) -> Result<Json<ResponseData<ServerErrorsData>>, ApiError> {
    // RSS sync failures, newest first, with the podcast title resolved (None if
    // the podcast vanished — FK cascade makes that only a read-time race).
    let rss_rows = podcast_sync_error::Entity::find()
        .order_by_desc(podcast_sync_error::Column::Id)
        .limit(MAX_ROWS)
        .all(&dbc)
        .await
        .map_err(db_err)?;
    let podcast_ids: Vec<i32> = rss_rows.iter().map(|r| r.podcast_id).collect();
    let podcast_titles: HashMap<i32, String> = podcast::Entity::find()
        .filter(podcast::Column::Id.is_in(podcast_ids))
        .all(&dbc)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|p| (p.id, p.title))
        .collect();
    let rss_sync = rss_rows
        .into_iter()
        .map(|r| PodcastSyncErrorData {
            id: r.id,
            podcast_id: r.podcast_id,
            podcast_title: podcast_titles.get(&r.podcast_id).cloned(),
            reason: r.reason,
            created_at: r.created_at,
        })
        .collect();

    // Download failures, newest first, with the episode title + parent podcast
    // resolved the same way.
    let dl_rows = episode_download_error::Entity::find()
        .order_by_desc(episode_download_error::Column::Id)
        .limit(MAX_ROWS)
        .all(&dbc)
        .await
        .map_err(db_err)?;
    let episode_ids: Vec<i32> = dl_rows.iter().map(|r| r.episode_id).collect();
    let episodes_by_id: HashMap<i32, (String, i32)> = episode::Entity::find()
        .filter(episode::Column::Id.is_in(episode_ids))
        .all(&dbc)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|e| (e.id, (e.title, e.podcast_id)))
        .collect();
    let episode_downloads = dl_rows
        .into_iter()
        .map(|r| {
            let found = episodes_by_id.get(&r.episode_id);
            EpisodeDownloadErrorData {
                id: r.id,
                episode_id: r.episode_id,
                episode_title: found.map(|(title, _)| title.clone()),
                podcast_id: found.map(|(_, pid)| *pid),
                reason: r.reason,
                created_at: r.created_at,
            }
        })
        .collect();

    Ok(Json(ResponseData::from_data(ServerErrorsData {
        rss_sync,
        episode_downloads,
    })))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn get_errors(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder()
            .method("GET")
            .uri("/api/v1/admin/server-errors");
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        router
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn server_errors_require_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(
            get_errors(&router, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn server_errors_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            get_errors(&router, Some(&user)).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn server_errors_admin_reads_persisted_rows() {
        let (_root, dbc, payload) = setup_test_db().await;

        // Seed one error of each kind against the fixture's podcast + episode.
        let podcast_id = payload["podcast_id"]
            .as_str()
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let episode_id = payload["episode_id"]
            .as_str()
            .unwrap()
            .parse::<i32>()
            .unwrap();
        halogen_orm::podcast_sync_error::Entity::record(
            &dbc,
            podcast_id,
            "Failed to fetch feed: connection refused".to_string(),
        )
        .await
        .expect("record sync error");
        halogen_orm::episode_download_error::Entity::record(
            &dbc,
            episode_id,
            "origin returned 404 (not found)".to_string(),
        )
        .await
        .expect("record download error");

        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
        let resp = get_errors(&router, Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;

        let rss = json["data"]["rss_sync"].as_array().expect("rss array");
        assert_eq!(rss.len(), 1);
        assert_eq!(rss[0]["podcast_id"].as_i64(), Some(podcast_id as i64));
        assert!(rss[0]["podcast_title"].is_string(), "title resolved");
        assert!(
            rss[0]["reason"]
                .as_str()
                .unwrap()
                .contains("connection refused")
        );

        let dls = json["data"]["episode_downloads"]
            .as_array()
            .expect("downloads array");
        assert_eq!(dls.len(), 1);
        assert_eq!(dls[0]["episode_id"].as_i64(), Some(episode_id as i64));
        assert!(dls[0]["episode_title"].is_string(), "title resolved");
        assert!(dls[0]["reason"].as_str().unwrap().contains("404"));
    }
}
