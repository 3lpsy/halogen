//! Shared router-test setup, JWT generation, and model factories keep IDs, claims, and defaults consistent across
//! resource suites. Authenticated request, JSON-body, and field-error helpers centralize oneshot/envelope handling.

use std::time::Duration as StdDuration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, header};
use axum::response::Response;
use chrono::Utc;
use halogen_migrations::connect_and_migrate;
use halogen_orm::{episode, playlist, podcast, podcast_config, user, user_podcast};
use halogen_wire::DownloadStatus;
use jsonwebtoken::{EncodingKey, Header};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection};
use time::{Duration, OffsetDateTime};

use crate::restart::RestartHandle;
use crate::routers::middleware::JwtClaims;
use halogen_config::Config;
use halogen_fixture::test_support::TestRoot;
use halogen_polling::PollingHandle;

/// The HS256 secret every test router signs and verifies with. The single source
/// of truth for the value `build_test_router` and `generate_jwt_token` share.
pub const TEST_JWT_SECRET: &str = "test-secret-key-for-jwt-signing";

/// Spin up a fresh migrated SQLite database under a temp [`TestRoot`]. The returned `root` is NOT yet marked
/// successful — the caller seeds it and calls `root.mark_success()` once seeding succeeds, preserving the
/// failure-sweep behaviour the per-resource harnesses relied on.
pub async fn new_test_db(suite: &str) -> (TestRoot, DatabaseConnection) {
    let root = TestRoot::new(suite);
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    (root, dbc)
}

/// Build the real application router wired to a test config (known JWT secret,
/// instant poll interval, no real restart handle).
pub fn build_test_router(dbc: DatabaseConnection) -> Router {
    build_test_router_with_media_root(dbc, Config::default().media_root)
}

/// Like [`build_test_router`] but with a caller-chosen `media_root`. The
/// audio/art file-serving endpoints confine served paths to `media_root`, so a
/// test that stages a real file for them must point the router at the directory
/// it stages into (e.g. its `TestRoot` dir).
pub fn build_test_router_with_media_root(
    dbc: DatabaseConnection,
    media_root: std::path::PathBuf,
) -> Router {
    let cfg = Config {
        auth_token_secret: TEST_JWT_SECRET.to_string(),
        auth_token_expiry_minutes: 60,
        media_root,
        ..Default::default()
    };
    let polling = PollingHandle::new(dbc.clone(), StdDuration::ZERO, 5);
    crate::routers::build_router(dbc, &cfg, polling, RestartHandle::new())
}

/// Mint a valid API (bearer) JWT for `user_id`, expiring in an hour.
pub fn generate_jwt_token(user_id: &str) -> String {
    encode_token(JwtClaims::api(user_id.to_string(), hour_from_now()))
}

/// Mint a media-scoped JWT — the value the `auth_media` cookie carries.
pub fn generate_media_jwt_token(user_id: &str) -> String {
    encode_token(JwtClaims::media(user_id.to_string(), hour_from_now()))
}

fn hour_from_now() -> usize {
    (OffsetDateTime::now_utc() + Duration::hours(1)).unix_timestamp() as usize
}

fn encode_token(claims: JwtClaims) -> String {
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .expect("encode token")
}

// ----- entity factories ------------------------------------------------------- Each returns a
// fully-populated `ActiveModel` (timestamps set, optional columns `None`) carrying sensible defaults. Callers
// override the fields a given test cares about before inserting, e.g. let mut p = podcast_model(id, owner_id);
// p.title = Set("Second Podcast".into()); p.insert(&dbc).await.unwrap();

/// A `user` row. `password` is bcrypt-hashed here.
pub fn user_model(id: i32, username: &str, password: &str, is_admin: bool) -> user::ActiveModel {
    let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST).expect("hash password");
    user::ActiveModel {
        id: Set(id),
        username: Set(username.to_string()),
        password_hash: Set(hashed),
        is_admin: Set(is_admin),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
}

/// Insert an admin + a regular user with the conventional test credentials and
/// return `(admin_id, user_id)`. Used by every resource harness that needs an
/// owner plus an admin.
pub async fn seed_admin_and_user(dbc: &DatabaseConnection, admin_id: i32, user_id: i32) {
    user_model(admin_id, "admin_user", "testadmin123", true)
        .insert(dbc)
        .await
        .expect("insert admin");
    user_model(user_id, "regular_user", "testuser123", false)
        .insert(dbc)
        .await
        .expect("insert user");
}

/// A `podcast` row owned by `owner_id`, all optional columns `None`.
pub fn podcast_model(id: i32, owner_id: i32) -> podcast::ActiveModel {
    podcast::ActiveModel {
        id: Set(id),
        title: Set("Test Podcast".to_string()),
        description: Set("Podcast description".to_string()),
        feed_url: Set("https://example.com/feed.xml".to_string()),
        art_url: Set(None),
        art_file_path: Set(None),
        author: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(owner_id),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
}

/// An `episode` row under `podcast_id`, `NotDownloaded`, all optional columns
/// `None` except a recent `published_at`.
pub fn episode_model(id: i32, podcast_id: i32) -> episode::ActiveModel {
    episode::ActiveModel {
        id: Set(id),
        podcast_id: Set(podcast_id),
        title: Set("Test Episode".to_string()),
        description: Set("Episode description".to_string()),
        content_url: Set("https://example.com/ep.mp3".to_string()),
        guid: Set(None),
        art_url: Set(None),
        published_at: Set(Some(Utc::now())),
        downloaded_at: Set(None),
        content_file_path: Set(None),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(DownloadStatus::NotDownloaded),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
}

/// A `playlist` row owned by `user_id` (non-default, position 0).
pub fn playlist_model(id: i32, user_id: i32) -> playlist::ActiveModel {
    playlist::ActiveModel {
        id: Set(id),
        name: Set("Test Playlist".to_string()),
        description: Set(None),
        user_id: Set(user_id),
        is_default: Set(false),
        position: Set(0),
        on_remove_delete_file_server: Set(false),
        on_remove_delete_file_client: Set(false),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
}

/// A `podcast_config` row with the conventional test knobs.
pub fn podcast_config_model(id: i32) -> podcast_config::ActiveModel {
    podcast_config::ActiveModel {
        id: Set(id),
        poll_interval_seconds: Set(Some(300)),
        max_episodes: Set(Some(100)),
        max_concurrent_downloads: Set(Some(3)),
        auto_download_enabled: Set(Some(false)),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
}

/// Subscribe `user_id` to `podcast_id` (the pivot row the read routes scope on).
pub async fn subscribe(dbc: &DatabaseConnection, user_id: i32, podcast_id: i32) {
    user_podcast::ActiveModel {
        user_id: Set(user_id),
        podcast_id: Set(podcast_id),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("subscribe user to podcast");
}

// ----- request / response helpers -------------------------------------------- The request/response plumbing
// every per-verb `mod tests` uses. A handful of tests still build requests inline where these don't fit —
// malformed/custom `Authorization` headers, `auth_media` cookies, CORS preflight, and helpers that take an
// optional token — which is expected.

/// A `Bearer`-authenticated request with an empty body (GET/DELETE).
pub fn authed(method: &str, uri: impl AsRef<str>, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request")
}

/// A `Bearer`-authenticated JSON request (POST/PUT). `body` is serialized as-is.
pub fn authed_json(
    method: &str,
    uri: impl AsRef<str>,
    token: &str,
    body: &serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request")
}

/// An unauthenticated JSON request (login, refresh, …).
pub fn json(method: &str, uri: impl AsRef<str>, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request")
}

/// An unauthenticated request with an empty body (logout, health probes, the
/// missing-credentials 401 paths).
pub fn unauthed(method: &str, uri: impl AsRef<str>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .body(Body::empty())
        .expect("build request")
}

/// Drain a response body and parse it as JSON.
pub async fn json_body(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("decode response")
}

/// The first validation-error message keyed under `field` in a response envelope
/// (`json["errors"][field][0]["message"]`).
pub fn field_error<'a>(json: &'a serde_json::Value, field: &str) -> &'a str {
    json["errors"][field][0]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error message under errors.{field}, got: {json}"))
}
