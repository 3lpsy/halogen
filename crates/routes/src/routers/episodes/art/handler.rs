use axum::{
    Extension,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::DatabaseConnection;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use std::path::{Path, PathBuf};

use crate::routers::auth::AuthConfig;
use crate::routers::episodes::MediaDownloadConfig;
use crate::routers::extractors::Id;
use crate::routers::guards;
use halogen_art::{ensure_episode_art, ensure_episode_art_small};

/// Serve full-resolution episode art through the server cache, fetching origin art on first use. Accept media cookies
/// or API bearer tokens. Missing artwork returns 204 so placeholders do not produce browser-console 404s.
pub async fn art(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(auth): Extension<AuthConfig>,
    Extension(cfg): Extension<MediaDownloadConfig>,
    Id(id): Id,
    req: Request,
) -> Response {
    if let Err(resp) = authorize(&dbc, &auth, id, req.headers()).await {
        return resp;
    }
    let path = match ensure_episode_art(&dbc, id, &cfg.media_root, true).await {
        Ok(Some(path)) => path,
        // No artwork anywhere for this row: 204 (not 404) so the browser doesn't
        // log a console error for the benign empty-art case. The `<img>` `onerror`
        // still fires on the empty 2xx body, so the UI shows its frown placeholder.
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!(episode_id = id, error = %e, "episode art unavailable");
            return (StatusCode::NOT_FOUND, "artwork unavailable").into_response();
        }
    };
    serve(&cfg.media_root, path, req).await
}

/// GET /episodes/{id}/art/small Like [`art`] but serves a downscaled (~256px) thumbnail variant, generated and
/// cached on first request. Used by the episode list, podcast list, mini player, and up-next thumbnails — call
/// sites that render at ~70px and must not pull the multi-MB original. Falls back to the original if the
/// thumbnail can't be made.
pub async fn art_small(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(auth): Extension<AuthConfig>,
    Extension(cfg): Extension<MediaDownloadConfig>,
    Id(id): Id,
    req: Request,
) -> Response {
    if let Err(resp) = authorize(&dbc, &auth, id, req.headers()).await {
        return resp;
    }
    let path = match ensure_episode_art_small(&dbc, id, &cfg.media_root, true).await {
        Ok(Some(path)) => path,
        // No artwork anywhere for this row: 204 (not 404) so the browser doesn't
        // log a console error for the benign empty-art case. The `<img>` `onerror`
        // still fires on the empty 2xx body, so the UI shows its frown placeholder.
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!(episode_id = id, error = %e, "episode art unavailable");
            return (StatusCode::NOT_FOUND, "artwork unavailable").into_response();
        }
    };
    serve(&cfg.media_root, path, req).await
}

/// Media auth (cookie/bearer) + subscription guard, shared by both art handlers.
/// `Err` carries the ready-to-return 401/404 response.
async fn authorize(
    dbc: &DatabaseConnection,
    auth: &AuthConfig,
    id: i32,
    headers: &axum::http::HeaderMap,
) -> Result<(), Response> {
    let user_id = crate::routers::media_auth::media_user_or_401(headers, auth)?;
    // Authorize before fetching/serving art: subscriber (or owner / admin) only,
    // mirroring the audio endpoint. A non-subscriber or missing episode 404s.
    let actor = guards::actor_from_id(dbc, user_id).await;
    // Trait-qualified: `ApiError` also has an inherent generic `into_response`.
    guards::require_episode_subscribed(dbc, actor, id)
        .await
        .map_err(IntoResponse::into_response)
}

/// Stream `path` with the day-long browser-cache header. Shared by both handlers. Defense-in-depth: the art
/// cache writes only under `media_root`, but confine the resolved path there before serving so a stray/legacy
/// `art_file_path` can't become an arbitrary-file read. Outside the root → 404 (same as "no art").
async fn serve(media_root: &Path, path: PathBuf, req: Request) -> Response {
    let path = match crate::routers::media_path::confined(media_root, &path) {
        Some(p) => p,
        None => {
            tracing::warn!(path = %path.display(), "refusing to serve art path outside media_root");
            return (StatusCode::NOT_FOUND, "artwork unavailable").into_response();
        }
    };
    match ServeFile::new(path).oneshot(req).await {
        Ok(resp) => {
            let mut resp = resp.into_response();
            // Art is auth'd (hence `private`) but stable per episode; a day of
            // browser caching avoids a revalidation round-trip per list item.
            resp.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("private, max-age=86400"),
            );
            resp
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
