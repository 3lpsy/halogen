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

/// GET /episodes/{id}/art
///
/// Serves the episode's full-resolution artwork from the server-side art cache,
/// fetching it from the origin `art_url` on first request (see `halogen_art`).
/// Clients never touch origin CDNs — `<img>` tags point here. Auth mirrors the
/// audio endpoint (media cookie for `<img src>`, bearer for programmatic
/// fetches). 204 when the episode has no artwork (the UI shows its placeholder;
/// 204 rather than 404 keeps the benign empty-art case out of the browser console).
/// Used by the full player + episode detail view, where the large image is wanted.
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

/// GET /episodes/{id}/art/small
///
/// Like [`art`] but serves a downscaled (~256px) thumbnail variant, generated and
/// cached on first request. Used by the episode list, podcast list, mini player,
/// and up-next thumbnails — call sites that render at ~70px and must not pull the
/// multi-MB original. Falls back to the original if the thumbnail can't be made.
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
mod tests {
    use crate::routers::episodes::tests::{
        build_test_router, build_test_router_with_media_root, generate_jwt_token, setup_test_db,
    };
    use crate::tests::harness::{authed, json_body, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    // No credential → 401 (same media auth as the audio endpoint).
    #[tokio::test]
    async fn test_art_unauthenticated_is_unauthorized() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

        let response = router
            .oneshot(unauthed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/art"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // Success path: a disk-cached art file is served with the day-long
    // Cache-Control header (the browser-cache contract). The file is pre-placed
    // and `art_file_path` set so `ensure_episode_art` short-circuits — no
    // network egress.
    #[tokio::test]
    async fn test_art_success_sets_cache_control() {
        use sea_orm::{ActiveModelTrait, ActiveValue};

        let (root, dbc, payload) = setup_test_db().await;
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let episode_id: i32 = payload
            .get("episode_id_1")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let art_path = root.path().join("episode_art.png");
        std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let update = halogen_orm::episode::ActiveModel {
            id: ActiveValue::set(episode_id),
            art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
            ..Default::default()
        };
        update.update(&dbc).await.unwrap();

        // The router serves only paths under media_root; stage into the root dir.
        let router = build_test_router_with_media_root(dbc, root.path().to_path_buf());
        let response = router
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/art"),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .expect("art response carries Cache-Control"),
            "private, max-age=86400"
        );
    }

    // /art/small route is wired and serves: a pre-placed (undecodable) original
    // means thumbnail generation falls back to the original, so the endpoint
    // still returns 200 with the day-long Cache-Control. Proves the new route +
    // export + shared serve/auth path without network egress or a real decode.
    #[tokio::test]
    async fn test_art_small_serves_with_cache_control() {
        use sea_orm::{ActiveModelTrait, ActiveValue};

        let (root, dbc, payload) = setup_test_db().await;
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let episode_id: i32 = payload
            .get("episode_id_1")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let art_path = root.path().join("episode_art.png");
        std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let update = halogen_orm::episode::ActiveModel {
            id: ActiveValue::set(episode_id),
            art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
            ..Default::default()
        };
        update.update(&dbc).await.unwrap();

        // The router serves only paths under media_root; stage into the root dir.
        let router = build_test_router_with_media_root(dbc, root.path().to_path_buf());
        let response = router
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/art/small"),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .expect("small art response carries Cache-Control"),
            "private, max-age=86400"
        );
    }

    // Bearer authenticates; the seeded episode has no `art_url`, so the cache
    // has nothing to fetch → 204 (proves route + auth + service wiring without
    // any network egress; 204 keeps the empty-art case out of the browser console).
    #[tokio::test]
    async fn test_art_bearer_no_artwork_is_no_content() {
        let (_root, dbc, payload) = setup_test_db().await;
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let router = build_test_router(dbc);

        let response = router
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/art"),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    // A bearer for a user subscribed to nothing must NOT fetch/serve the episode's
    // art — the subscription guard 404s before any art work. The validation-error
    // body (code "exists") distinguishes it from the "no artwork" 204.
    #[tokio::test]
    async fn test_art_hidden_from_non_subscriber() {
        let (_root, dbc, payload) = setup_test_db().await;
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
        let token = generate_jwt_token(&(i32::MAX - 500).to_string());
        let router = build_test_router(dbc);

        let response = router
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/art"),
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let json = json_body(response).await;
        let code = json
            .get("errors")
            .and_then(|e| e.get("id"))
            .and_then(|i| i.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("code"))
            .and_then(|c| c.as_str())
            .expect("subscription guard error code");
        assert_eq!(code, "exists", "non-subscriber must not read episode art");
    }
}
