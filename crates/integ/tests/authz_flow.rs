//! Authorization contract — what's admin-gated, what's owner-gated, and what's
//! open to any authed user. Pinned here so any future change is visible.
//!
//! - `JwtAuthLayer` gates every `/api/v1` resource route on *authentication*.
//! - Control routes (`/admin/config`, `/admin/poll`, `/admin/start`, `/admin/stop`) are admin-only
//!   (the `AdminUser` extractor).
//! - Resource *writes* are owner-or-admin: PUT/DELETE podcast, PUT/DELETE
//!   playlist + membership, POST/PUT/DELETE episode (by the parent podcast's
//!   owner), and podcast-config writes — PUT /podcast-configs/{id} and the nested
//!   POST/DELETE /podcasts/{id}/config. A non-owner non-admin gets 403.
//! - GET /podcast-configs/{id} is owner-or-admin (an unowned config → 403 for a
//!   non-owner), so a config read can't leak another owner's settings. There is
//!   no standalone create/list/delete — configs live under their podcast.
//! - Open to any authed user: the create endpoints (POST /podcasts, /playlists).
//! - `/users`: admin-gated. List + read-other + delete are admin-only; a
//!   non-admin may self-update plain fields but not flip its own admin bit.
//!
//! Driven over raw HTTP (not the typed `ApiClient`) so we can assert exact status
//! codes on the wire for arbitrary methods/bodies, including routes the typed
//! client doesn't expose (`POST /episodes`, `DELETE /episodes/{id}`).
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(authz_flow)'`

use halogen_integ::*;
use reqwest::StatusCode;

/// Base API URL (`…/api/v1`).
fn api_url(app: &TestApp, path: &str) -> String {
    format!("{}/api/v1{}", app.base_url, path)
}

/// Send a request with an optional bearer token, returning the status code.
async fn status(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> u16 {
    let mut rb = http.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    rb.send().await.expect("request").status().as_u16()
}

/// The admin-gated control routes reject a NON-admin authed user (403), while
/// rejecting anonymous (401). Pins the admin-only control surface.
#[tokio::test]
async fn admin_only_routes_gate_non_admin() {
    let app = spawn().await;
    let _admin = app.seed_admin().await;
    let (_uid, user_token) = app.seed_user("plain").await;
    let http = reqwest::Client::new();

    // The AdminUser-gated control set, all under the `/admin` prefix.
    let cases: &[(reqwest::Method, &str)] = &[
        (reqwest::Method::GET, "/admin/config"),
        (reqwest::Method::POST, "/admin/poll"),
        (reqwest::Method::POST, "/admin/start"),
        (reqwest::Method::POST, "/admin/stop"),
    ];

    for (method, path) in cases {
        // Anonymous → 401 (auth layer).
        assert_eq!(
            status(&http, method.clone(), &api_url(&app, path), None, None).await,
            401,
            "{method} {path}: anonymous must be 401"
        );
        // Non-admin authed → 403 (AdminUser extractor in the handler).
        assert_eq!(
            status(
                &http,
                method.clone(),
                &api_url(&app, path),
                Some(&user_token),
                None,
            )
            .await,
            403,
            "{method} {path}: non-admin must be 403 (admin-gated)"
        );
    }

    // `/status` is the read any authed user may do — NOT admin-gated.
    assert_eq!(
        status(
            &http,
            reqwest::Method::GET,
            &api_url(&app, "/status"),
            None,
            None
        )
        .await,
        401,
        "GET /status: anonymous must be 401"
    );
    assert_eq!(
        status(
            &http,
            reqwest::Method::GET,
            &api_url(&app, "/status"),
            Some(&user_token),
            None,
        )
        .await,
        200,
        "GET /status: any authed user may read polling status"
    );
}

/// Resource WRITES require owner-or-admin. The seed helpers attribute ownership
/// to the admin/first user, so a separate non-admin "plain" user is neither owner
/// nor admin: it gets 403 mutating those resources (and 401 when anonymous). The
/// open create endpoints stay 2xx; the /users routes are admin-gated (a non-admin
/// may only self-update plain fields).
#[tokio::test]
async fn resource_writes_require_owner_or_admin() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let (other_user_id, user_token) = app.seed_user("plain").await;
    let http = reqwest::Client::new();

    // Resources owned by the admin (seed helpers attribute ownership to the
    // admin/first user). The episode lives under a SEPARATE podcast so deleting
    // one fixture doesn't cascade another.
    let podcast_id = app.seed_podcast("Authz", "https://feed.test/authz").await;
    let ep_podcast = app.seed_podcast("EpHome", "https://feed.test/ephome").await;
    let episode_id = app
        .seed_episode(
            ep_podcast,
            "Ep",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;
    let playlist_id = app.seed_playlist("PL", true).await;

    let ok = |s: u16| (200..300).contains(&s);

    // ── users: admin-gated. A non-admin may self-update plain fields only; it
    //    may not read/update another user, escalate its own admin bit, or delete
    //    anyone. Listing + delete are admin-only. ──
    assert_eq!(
        status(
            &http,
            reqwest::Method::PUT,
            &api_url(&app, &format!("/users/{other_user_id}")),
            None,
            Some(serde_json::json!({"data": {"username": "x"}})),
        )
        .await,
        401,
        "PUT /users/{{id}}: anonymous must be 401"
    );

    // Non-admin updating a DIFFERENT user (the admin) → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::PUT,
            &api_url(&app, &format!("/users/{}", admin.id)),
            Some(&user_token),
            Some(serde_json::json!({"data": {"username": "renamed_by_peer"}})),
        )
        .await,
        403,
        "PUT /users/{{id}}: non-admin may not update another user"
    );

    // Non-admin self-update of a plain field → allowed.
    let s = status(
        &http,
        reqwest::Method::PUT,
        &api_url(&app, &format!("/users/{other_user_id}")),
        Some(&user_token),
        Some(serde_json::json!({"data": {"username": "renamed_self"}})),
    )
    .await;
    assert!(
        ok(s),
        "PUT /users/{{id}}: non-admin self-update is allowed (got {s})"
    );

    // Non-admin may NOT escalate itself to admin → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::PUT,
            &api_url(&app, &format!("/users/{other_user_id}")),
            Some(&user_token),
            Some(serde_json::json!({"data": {"is_admin": true}})),
        )
        .await,
        403,
        "PUT /users/{{id}}: non-admin may not grant itself admin"
    );

    // Delete is admin-only (under the `/admin` prefix): non-admin → 403,
    // admin → 2xx.
    let (throwaway_id, _t) = app.seed_user("throwaway").await;
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/admin/users/{throwaway_id}")),
            Some(&user_token),
            None,
        )
        .await,
        403,
        "DELETE /admin/users/{{id}}: non-admin may not delete users"
    );
    let s = status(
        &http,
        reqwest::Method::DELETE,
        &api_url(&app, &format!("/admin/users/{throwaway_id}")),
        Some(&admin.token),
        None,
    )
    .await;
    assert!(
        ok(s),
        "DELETE /admin/users/{{id}}: admin may delete (got {s})"
    );

    // ── podcasts: create OPEN; delete OWNER-gated ──
    let create_podcast = serde_json::json!({
        "data": {"title": "By Peer", "feed_url": "https://feed.test/peer"}
    });
    assert_eq!(
        status(
            &http,
            reqwest::Method::POST,
            &api_url(&app, "/podcasts"),
            None,
            Some(create_podcast.clone()),
        )
        .await,
        401,
        "POST /podcasts: anonymous must be 401"
    );
    let s = status(
        &http,
        reqwest::Method::POST,
        &api_url(&app, "/podcasts"),
        Some(&user_token),
        Some(create_podcast),
    )
    .await;
    assert!(ok(s), "POST /podcasts by any authed user is open (got {s})");
    // Deleting the admin-owned podcast as the non-owner → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/podcasts/{podcast_id}")),
            None,
            None,
        )
        .await,
        401,
        "DELETE /podcasts/{{id}}: anonymous must be 401"
    );
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/podcasts/{podcast_id}")),
            Some(&user_token),
            None,
        )
        .await,
        403,
        "DELETE /podcasts/{{id}} by a non-owner must be 403"
    );

    // ── playlists: create OPEN; update/delete/membership OWNER-gated ──
    // `is_default: true` because the plain user has no playlists yet and the
    // per-user rule requires the first one to be the default queue.
    let create_pl = serde_json::json!({"data": {"name": "Peer List", "is_default": true}});
    assert_eq!(
        status(
            &http,
            reqwest::Method::POST,
            &api_url(&app, "/playlists"),
            None,
            Some(create_pl.clone()),
        )
        .await,
        401,
        "POST /playlists: anonymous must be 401"
    );
    let s = status(
        &http,
        reqwest::Method::POST,
        &api_url(&app, "/playlists"),
        Some(&user_token),
        Some(create_pl),
    )
    .await;
    assert!(
        ok(s),
        "POST /playlists by any authed user is open (got {s})"
    );
    // Update the admin-owned playlist as the non-owner → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::PUT,
            &api_url(&app, &format!("/playlists/{playlist_id}")),
            Some(&user_token),
            Some(serde_json::json!({"data": {"name": "Renamed By Peer"}})),
        )
        .await,
        403,
        "PUT /playlists/{{id}} by a non-owner must be 403"
    );
    // Add an episode to the admin-owned playlist as the non-owner → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::POST,
            &api_url(
                &app,
                &format!("/playlists/{playlist_id}/episodes/{episode_id}"),
            ),
            Some(&user_token),
            Some(serde_json::json!({
                "data": {"playlist_id": playlist_id, "episode_id": episode_id}
            })),
        )
        .await,
        403,
        "POST membership by a non-owner must be 403"
    );
    // Delete the admin-owned playlist as the non-owner → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/playlists/{playlist_id}")),
            None,
            None,
        )
        .await,
        401,
        "DELETE /playlists/{{id}}: anonymous must be 401"
    );
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/playlists/{playlist_id}")),
            Some(&user_token),
            None,
        )
        .await,
        403,
        "DELETE /playlists/{{id}} by a non-owner must be 403"
    );

    // ── episodes: create + delete gated by the parent podcast's owner ──
    let fresh_podcast = app.seed_podcast("Fresh", "https://feed.test/fresh").await;
    let create_ep = serde_json::json!({
        "data": {
            "podcast_id": fresh_podcast,
            "title": "Peer Episode",
            "description": "A peer-created episode.",
            "content_url": "https://example.test/peer.mp3"
        }
    });
    assert_eq!(
        status(
            &http,
            reqwest::Method::POST,
            &api_url(&app, "/episodes"),
            None,
            Some(create_ep.clone()),
        )
        .await,
        401,
        "POST /episodes: anonymous must be 401"
    );
    // `fresh_podcast` is admin-owned → a non-owner creating an episode under it → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::POST,
            &api_url(&app, "/episodes"),
            Some(&user_token),
            Some(create_ep),
        )
        .await,
        403,
        "POST /episodes under a non-owned podcast must be 403"
    );
    // Delete an admin-owned episode as the non-owner → 403.
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/episodes/{episode_id}")),
            None,
            None,
        )
        .await,
        401,
        "DELETE /episodes/{{id}}: anonymous must be 401"
    );
    assert_eq!(
        status(
            &http,
            reqwest::Method::DELETE,
            &api_url(&app, &format!("/episodes/{episode_id}")),
            Some(&user_token),
            None,
        )
        .await,
        403,
        "DELETE /episodes/{{id}} by a non-owner must be 403"
    );

    // Sanity: the admin token still works on an admin-gated route, proving the
    // 403s above were about ownership/role, not a broken fixture.
    assert_eq!(
        status(
            &http,
            reqwest::Method::GET,
            &api_url(&app, "/admin/config"),
            Some(&admin.token),
            None,
        )
        .await,
        StatusCode::OK.as_u16(),
        "admin may GET /admin/config"
    );
}
