use axum::{
    Extension,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use halogen_orm::episode::{Column, Entity as EpisodeEntity};
use halogen_wire::DownloadStatus;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use std::path::Path;

use crate::routers::auth::AuthConfig;
use crate::routers::episodes::MediaDownloadConfig;
use crate::routers::extractors::Id;
use crate::routers::guards;

/// Serve downloaded episode audio with Range support. Browser cookies must contain media-scoped tokens; programmatic
/// bearer headers must contain normal API tokens. Reject cross-use and return 404 when no server download exists.
pub async fn audio(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(auth): Extension<AuthConfig>,
    Extension(cfg): Extension<MediaDownloadConfig>,
    Id(id): Id,
    req: Request,
) -> Response {
    // Authenticate the media credential (bearer-or-cookie, scope-locked) and
    // recover the user id — see `media_auth`.
    let user_id = match crate::routers::media_auth::media_user_or_401(req.headers(), &auth) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // Authorize: only a subscriber (or owner / admin) may stream the episode — the
    // same access the JSON episode routes grant. A non-subscriber or missing
    // episode 404s (existence stays hidden). Runs before any file work.
    let actor = guards::actor_from_id(&dbc, user_id).await;
    if let Err(e) = guards::require_episode_subscribed(&dbc, actor, id).await {
        // Trait-qualified: `ApiError` also has an inherent generic `into_response`.
        return IntoResponse::into_response(e);
    }

    let episode = match EpisodeEntity::find()
        .filter(Column::Id.eq(id))
        .one(&dbc)
        .await
    {
        Ok(Some(ep)) => ep,
        Ok(None) => return (StatusCode::NOT_FOUND, "episode not found").into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Only serve episodes the server has downloaded to disk.
    let stored = match (episode.download_status, episode.content_file_path) {
        (DownloadStatus::Downloaded, Some(path)) => path,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                "episode not downloaded on the server",
            )
                .into_response();
        }
    };

    // Defense-in-depth: `content_file_path` is server-managed (only the download pipeline writes it, under
    // `media_root`) and clients can no longer set it — but confirm the resolved path stays inside `media_root`
    // before serving, so a stray/legacy/hand-edited absolute path can never become an arbitrary-file read. A
    // path outside the root is treated as "not downloaded" (404).
    let path = match crate::routers::media_path::confined_or_rebased(
        &cfg.media_root,
        Path::new(&stored),
    ) {
        Some(p) => p,
        None => {
            tracing::warn!(
                episode_id = id,
                path = %stored,
                "refusing to serve audio path outside media_root"
            );
            return (
                StatusCode::NOT_FOUND,
                "episode not downloaded on the server",
            )
                .into_response();
        }
    };

    // `ServeFile` handles Range requests, content-type sniffing, and a missing
    // file (404). The inner service is infallible.
    match ServeFile::new(path).oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
