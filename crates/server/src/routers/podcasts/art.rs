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
use halogen_art::{ensure_podcast_art, ensure_podcast_art_small};

/// GET /podcasts/{id}/art
///
/// Serves the podcast's full-resolution artwork from the server-side art cache,
/// fetching it from the origin `art_url` on first request (see `halogen_art`).
/// Clients never touch origin CDNs — `<img>` tags point here. Auth mirrors the
/// audio endpoint (media cookie for `<img src>`, bearer for programmatic
/// fetches). 204 when the podcast has no artwork (the UI shows its placeholder;
/// 204 rather than 404 keeps the benign empty-art case out of the browser console).
/// Used by the podcast detail view, where the large image is wanted.
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
    let path = match ensure_podcast_art(&dbc, id, &cfg.media_root, true).await {
        Ok(Some(path)) => path,
        // No artwork anywhere for this row: 204 (not 404) so the browser doesn't
        // log a console error for the benign empty-art case. The `<img>` `onerror`
        // still fires on the empty 2xx body, so the UI shows its frown placeholder.
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!(podcast_id = id, error = %e, "podcast art unavailable");
            return (StatusCode::NOT_FOUND, "artwork unavailable").into_response();
        }
    };
    serve(&cfg.media_root, path, req).await
}

/// GET /podcasts/{id}/art/small
///
/// Like [`art`] but serves a downscaled (~256px) thumbnail variant, generated and
/// cached on first request. Used by the podcast list — call sites that render at
/// ~70px and must not pull the multi-MB original. Falls back to the original if
/// the thumbnail can't be made.
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
    let path = match ensure_podcast_art_small(&dbc, id, &cfg.media_root, true).await {
        Ok(Some(path)) => path,
        // No artwork anywhere for this row: 204 (not 404) so the browser doesn't
        // log a console error for the benign empty-art case. The `<img>` `onerror`
        // still fires on the empty 2xx body, so the UI shows its frown placeholder.
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!(podcast_id = id, error = %e, "podcast art unavailable");
            return (StatusCode::NOT_FOUND, "artwork unavailable").into_response();
        }
    };
    serve(&cfg.media_root, path, req).await
}

/// Media auth (cookie/bearer) + subscription guard, shared by both art handlers.
/// `id` is the podcast id; a non-subscriber or missing podcast 404s (existence
/// stays hidden). `Err` carries the ready-to-return 401/404 response.
async fn authorize(
    dbc: &DatabaseConnection,
    auth: &AuthConfig,
    id: i32,
    headers: &axum::http::HeaderMap,
) -> Result<(), Response> {
    let user_id = crate::routers::media_auth::media_user_or_401(headers, auth)?;
    let actor = guards::actor_from_id(dbc, user_id).await;
    // Trait-qualified: `ApiError` also has an inherent generic `into_response`.
    guards::require_subscribed(dbc, actor, id)
        .await
        .map_err(IntoResponse::into_response)
}

/// Stream `path` with the day-long browser-cache header. Shared by both handlers.
///
/// Defense-in-depth: the art cache writes only under `media_root`, but confine
/// the resolved path there before serving so a stray/legacy `art_file_path`
/// can't become an arbitrary-file read. Outside the root → 404 (same as "no art").
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
            // Art is auth'd (hence `private`) but stable per podcast; a day of
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
