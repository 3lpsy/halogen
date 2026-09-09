//! Real Axum server on an ephemeral port with migrated temporary SQLite and seed/JWT helpers, shared by HTTP and
//! browser tests. Only outbound RSS/audio/discovery providers are mocked; routing, middleware, DB, and JWT use
//! production code.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use tokio::net::TcpListener;

use halogen_config::Config;
use halogen_polling::PollingHandle;
use halogen_server::routers::middleware::JwtClaims;

use crate::test_fixture::TestRoot;

/// JWT signing secret used by the test server. Helpers sign tokens with the
/// same value so requests pass the real `JwtAuthLayer`.
const TEST_JWT_SECRET: &str = "test-secret-key-for-jwt-signing";

/// A running test server. Holds the bound address, a DB handle for direct seeding, and the temp-dir guard
/// (cleaned on drop). A minimal valid 2×2 RGBA PNG — real, decodable image bytes for
/// [`TestApp::seed_podcast_art`] (the `/art/small` thumbnail generator must be able to downscale it, so a
/// degenerate 0-byte or 1×1 file won't do).
const PNG_2X2: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xef, 0xe0, 0xf0,
    0x1f, 0x84, 0x19, 0x60, 0x0c, 0x00, 0x59, 0xca, 0x09, 0xf9, 0x9f, 0xbc, 0x6c, 0x86, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

pub struct TestApp {
    pub addr: SocketAddr,
    pub base_url: String,
    pub dbc: sea_orm::DatabaseConnection,
    /// Where the server stores downloaded media (the test temp dir). Playback
    /// journeys stage real bytes here via [`TestApp::download_on_server`].
    media_root: PathBuf,
    jwt_secret: String,
    _root: TestRoot,
}

/// Credentials returned by [`TestApp::seed_admin`].
pub struct AdminCreds {
    pub id: i32,
    pub username: String,
    pub password: String,
    /// A ready-to-use bearer token for this admin.
    pub token: String,
}

/// Options for [`spawn_with`].
#[derive(Default)]
pub struct SpawnOptions {
    /// If set, the server mounts a public directory (frontend `dist/`) as the
    /// `/` fallback — required for browser E2E tests.
    pub public_root: Option<PathBuf>,
    /// Enable the mock download service: `POST /episodes/{id}/download` copies
    /// the `data/tests/nasa-test-clip.mp3` fixture into the test's `media_root`
    /// instead of fetching the (fake) enclosure URL. Lets download/playback
    /// journeys complete a REAL byte flow without a network mock.
    pub use_mock_download: bool,
    /// Override the Discover iTunes provider's upstream search endpoint (point it
    /// at a wiremock server). `None` keeps the real-host default.
    pub discover_itunes_base_url: Option<String>,
    /// Override the Discover gpodder provider's upstream search endpoint. `None`
    /// keeps the real-host default.
    pub discover_gpodder_base_url: Option<String>,
}

/// Spawn a real axum server on `127.0.0.1:0` with a fresh migrated SQLite DB.
/// The server runs on a background task for the lifetime of the returned
/// [`TestApp`].
pub async fn spawn() -> TestApp {
    spawn_with(SpawnOptions::default()).await
}

/// Install a stderr `tracing` subscriber once per test process so the in-process server's logs land in
/// nextest's captured output (shown on failure). nextest runs each test in its own process, so a `OnceLock`
/// guard is enough. Verbosity defaults to `info` for our crates + `warn` for noisy deps; override with
/// `RUST_LOG` (e.g. `RUST_LOG=halogen_server=debug`).
fn init_test_tracing() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,hyper=warn,hyper_util=warn,sqlx=warn,sea_orm=warn,reqwest=warn")
        });
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}

/// Like [`spawn`] but with explicit [`SpawnOptions`] (e.g. a public dir).
pub async fn spawn_with(opts: SpawnOptions) -> TestApp {
    init_test_tracing();

    let mut root = TestRoot::new("integration");
    let db_path = root.path().join("halogen.db");

    // Prod parity: `main` opens WAL by default (no test sets `db_no_wal`),
    // so Tier-1 runs on the same journal profile as production.
    let dbc = halogen_migrations::connect_and_migrate_wal(&db_path, true)
        .await
        .expect("connect_and_migrate test db");

    let media_root = root.path().join("media");

    let mut cfg = Config {
        auth_token_secret: TEST_JWT_SECRET.to_string(),
        auth_token_expiry_minutes: 60,
        enable_public_server: opts.public_root.is_some(),
        public_root: opts.public_root,
        public_url_path: "/".to_string(),
        // Tests seed their own playlists and assert exact counts — keep the
        // startup "Queue" seed out of the way.
        db_skip_default_playlist: true,
        // Media lives in the test temp dir (cleaned on drop), never the
        // repo-relative default — mock downloads would otherwise dirty the repo.
        media_root: media_root.clone(),
        dev_use_mock_download: opts.use_mock_download,
        ..Default::default()
    };
    // Missing provider mocks fail before opening a socket. Tests never use production directories.
    cfg.discover_itunes_base_url = "disabled://discover/itunes".into();
    cfg.discover_gpodder_base_url = "disabled://discover/gpodder".into();
    if let Some(url) = opts.discover_itunes_base_url {
        cfg.discover_itunes_base_url = url;
    }
    if let Some(url) = opts.discover_gpodder_base_url {
        cfg.discover_gpodder_base_url = url;
    }

    // Zero interval => no background ticking; tests drive polling explicitly.
    let polling = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    let router = halogen_server::routers::build_router(
        dbc.clone(),
        &cfg,
        polling,
        halogen_server::restart::RestartHandle::new(),
    );

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server crashed");
    });

    root.mark_success();

    TestApp {
        base_url: format!("http://{addr}"),
        addr,
        dbc,
        media_root,
        jwt_secret: TEST_JWT_SECRET.to_string(),
        _root: root,
    }
}

impl TestApp {
    /// Absolute URL for a server path, e.g. `app.url("/episodes")`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Mint a JWT for `user_id` using the server's auth secret.
    pub fn jwt(&self, user_id: &str) -> String {
        use jsonwebtoken::{EncodingKey, Header};

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize;
        let claims = JwtClaims::api(user_id.to_string(), exp);
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .expect("encode jwt")
    }

    /// Mint a media-scoped JWT for `user_id` (the value the `auth_media` cookie
    /// carries) using the server's auth secret.
    pub fn media_jwt(&self, user_id: &str) -> String {
        use jsonwebtoken::{EncodingKey, Header};

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize;
        let claims = JwtClaims::media(user_id.to_string(), exp);
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .expect("encode media jwt")
    }

    /// Seed an admin user and return its credentials + bearer token.
    pub async fn seed_admin(&self) -> AdminCreds {
        halogen_fixture::user::seed_admin_user(&self.dbc, "admin", Some("adminpw123"))
            .await
            .expect("seed admin");

        use halogen_orm::user::{Column, Entity as UserEntity};
        let admin = UserEntity::find()
            .filter(Column::IsAdmin.eq(true))
            .one(&self.dbc)
            .await
            .expect("query admin")
            .expect("admin exists");

        AdminCreds {
            token: self.jwt(&admin.id.to_string()),
            id: admin.id,
            username: admin.username,
            password: "adminpw123".to_string(),
        }
    }

    /// Insert a non-admin user straight into the DB and return `(id, token)`.
    /// Mirrors what a provisioning path would write (a bcrypt hash, not a
    /// plaintext password). Used to exercise update/delete on a user other than
    /// the authed admin, and admin-only authorization (403) paths.
    pub async fn seed_user(&self, username: &str) -> (i32, String) {
        use halogen_orm::user::ActiveModel as UserActiveModel;
        use std::sync::atomic::{AtomicI32, Ordering};

        // Distinct, descending ids just below the admin's `i32::MAX`. A fixed id
        // collided when a test called `seed_user` twice; auto-increment instead
        // runs *past* admin's `i32::MAX` and overflows i32 on read-back. nextest
        // runs each test in its own process, so this counter starts fresh per test.
        static NEXT_USER_ID: AtomicI32 = AtomicI32::new(i32::MAX - 1);
        let id = NEXT_USER_ID.fetch_sub(1, Ordering::Relaxed);

        let now = chrono::Utc::now();
        let user = UserActiveModel {
            id: ActiveValue::set(id),
            username: ActiveValue::set(username.to_string()),
            password_hash: ActiveValue::set(
                "$2b$12$............................................".to_string(),
            ),
            is_admin: ActiveValue::set(false),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
        };
        let model = user.insert(&self.dbc).await.expect("insert extra user");
        let token = self.jwt(&model.id.to_string());
        (model.id, token)
    }

    /// Insert a SECOND loginable non-admin user with a real bcrypt password hash, returning `(id, token)`.
    /// Unlike [`Self::seed_user`] (placeholder hash, cannot authenticate), this account can sign in through the
    /// UI — used by the multi-account e2e journey's add-account flow. Ids descend from just below the admin's
    /// `i32::MAX`, distinct from `seed_user`'s.
    pub async fn seed_login_user(&self, username: &str, password: &str) -> (i32, String) {
        use std::sync::atomic::{AtomicI32, Ordering};

        // A separate counter range from `seed_user` (starts higher-down) so the
        // two helpers can be used in the same test without id collisions.
        static NEXT_LOGIN_ID: AtomicI32 = AtomicI32::new(i32::MAX - 1000);
        let id = NEXT_LOGIN_ID.fetch_sub(1, Ordering::Relaxed);
        halogen_fixture::user::seed_password_user(&self.dbc, id, username, password, false)
            .await
            .expect("seed loginable user");
        (id, self.jwt(&id.to_string()))
    }

    /// The admin (or, failing that, the first) user's id — the account the flows
    /// act as. Used to attribute seeded ownership / subscriptions / listen-state.
    async fn owner_user_id(&self) -> Option<i32> {
        use halogen_orm::user::{Column, Entity as UserEntity};
        if let Ok(Some(admin)) = UserEntity::find()
            .filter(Column::IsAdmin.eq(true))
            .one(&self.dbc)
            .await
        {
            return Some(admin.id);
        }
        UserEntity::find()
            .one(&self.dbc)
            .await
            .ok()
            .flatten()
            .map(|u| u.id)
    }

    /// Like [`Self::owner_user_id`] but seeds a minimal user when none exists yet,
    /// so content seeded before any `seed_admin`/`seed_user` still has a valid
    /// owner (the `owner_id`/`user_id` foreign keys are enforced).
    async fn ensure_owner_user_id(&self) -> i32 {
        if let Some(id) = self.owner_user_id().await {
            return id;
        }
        use halogen_orm::user::ActiveModel;
        let now = chrono::Utc::now();
        ActiveModel {
            id: ActiveValue::NotSet,
            username: ActiveValue::set("seed_owner".to_string()),
            password_hash: ActiveValue::set("x".to_string()),
            is_admin: ActiveValue::set(false),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
        }
        .insert(&self.dbc)
        .await
        .expect("seed owner user")
        .id
    }

    /// Seed a podcast whose `feed_url` points at a mock server. Returns the podcast id. This is the *only*
    /// place upstream is faked — the server will fetch the feed from `feed_url` during a poll. The podcast is
    /// owned by and subscribed to the admin/first user, so the owner's subscription-scoped library lists it
    /// (call `seed_admin` first).
    pub async fn seed_podcast(&self, title: &str, feed_url: &str) -> i32 {
        use halogen_orm::podcast::ActiveModel as PodcastActiveModel;
        use halogen_orm::user_podcast::ActiveModel as UserPodcastActiveModel;

        let now = chrono::Utc::now();
        let owner = self.ensure_owner_user_id().await;
        let podcast = PodcastActiveModel {
            title: ActiveValue::set(title.to_string()),
            description: ActiveValue::set(format!("{title} description")),
            feed_url: ActiveValue::set(feed_url.to_string()),
            art_url: ActiveValue::set(None),
            art_file_path: ActiveValue::set(None),
            author: ActiveValue::set(None),
            etag: ActiveValue::set(None),
            last_modified: ActiveValue::set(None),
            polled_at: ActiveValue::set(None),
            podcast_config_id: ActiveValue::set(None),
            owner_id: ActiveValue::set(owner),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
            ..Default::default()
        };
        let model = podcast.insert(&self.dbc).await.expect("insert podcast");
        UserPodcastActiveModel {
            user_id: ActiveValue::set(owner),
            podcast_id: ActiveValue::set(model.id),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
        }
        .insert(&self.dbc)
        .await
        .expect("subscribe owner to podcast");
        model.id
    }

    /// Bulk-insert `n` episodes for `podcast_id`, newest first (`published_at`
    /// descends with the index, titles zero-padded so lexical == chronological).
    /// Returns the inserted ids in creation order. Used by pagination /
    /// infinite-scroll tests that need more than one server page of data.
    pub async fn seed_episodes(&self, podcast_id: i32, n: usize) -> Vec<i32> {
        use halogen_orm::episode::ActiveModel as EpisodeActiveModel;

        let now = chrono::Utc::now();
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            let episode = EpisodeActiveModel {
                podcast_id: ActiveValue::set(podcast_id),
                title: ActiveValue::set(format!("Episode {i:04}")),
                description: ActiveValue::set(format!("Description for episode {i}")),
                content_url: ActiveValue::set(format!("https://example.test/ep/{i}.mp3")),
                art_url: ActiveValue::set(None),
                published_at: ActiveValue::set(Some(now - chrono::Duration::minutes(i as i64))),
                downloaded_at: ActiveValue::set(None),
                content_file_path: ActiveValue::set(None),
                art_file_path: ActiveValue::set(None),
                download_status: ActiveValue::set(Default::default()),
                duration_secs: ActiveValue::set(Some(1800)),
                created_at: ActiveValue::set(now),
                updated_at: ActiveValue::set(now),
                ..Default::default()
            };
            let model = episode.insert(&self.dbc).await.expect("insert episode");
            ids.push(model.id);
        }
        ids
    }

    /// Insert one episode with explicit title/description/podcast/playback status
    /// and a caller-chosen `published_at`, returning its id. Lets filter/search/
    /// order journeys build a precise, distinct dataset.
    pub async fn seed_episode(
        &self,
        podcast_id: i32,
        title: &str,
        description: &str,
        published_at: chrono::DateTime<chrono::Utc>,
        playback_status: halogen_wire::PlaybackStatus,
    ) -> i32 {
        use halogen_orm::episode::ActiveModel as EpisodeActiveModel;

        let episode = EpisodeActiveModel {
            podcast_id: ActiveValue::set(podcast_id),
            title: ActiveValue::set(title.to_string()),
            description: ActiveValue::set(description.to_string()),
            content_url: ActiveValue::set(format!("https://example.test/{title}.mp3")),
            published_at: ActiveValue::set(Some(published_at)),
            created_at: ActiveValue::set(published_at),
            updated_at: ActiveValue::set(published_at),
            duration_secs: ActiveValue::set(Some(1800)),
            ..Default::default()
        };
        let id = episode.insert(&self.dbc).await.expect("insert episode").id;
        // Listen state is per-user now; attribute the requested status to the
        // admin/first user (the account the flows act as).
        self.set_playback_status(&[id], playback_status).await;
        id
    }

    /// Insert a playlist and return its id. Pass `is_default = true` for the
    /// queue-backing playlist (the UI resolves the queue via the `is_default`
    /// flag / `GET /playlists/default`, not a fixed id).
    pub async fn seed_playlist(&self, name: &str, is_default: bool) -> i32 {
        use halogen_orm::playlist::ActiveModel as PlaylistActiveModel;

        let now = chrono::Utc::now();
        let owner = self.ensure_owner_user_id().await;
        let playlist = PlaylistActiveModel {
            name: ActiveValue::set(name.to_string()),
            description: ActiveValue::set(None),
            user_id: ActiveValue::set(owner),
            is_default: ActiveValue::set(is_default),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
            ..Default::default()
        };
        playlist
            .insert(&self.dbc)
            .await
            .expect("insert playlist")
            .id
    }

    /// Add `episode_ids` to `playlist_id` in order (0-based `position`).
    pub async fn seed_playlist_episodes(&self, playlist_id: i32, episode_ids: &[i32]) {
        use halogen_orm::episode_playlist::ActiveModel as EpisodePlaylistActiveModel;

        let now = chrono::Utc::now();
        for (position, &episode_id) in episode_ids.iter().enumerate() {
            let row = EpisodePlaylistActiveModel {
                episode_id: ActiveValue::set(episode_id),
                playlist_id: ActiveValue::set(playlist_id),
                position: ActiveValue::set(position as i32),
                created_at: ActiveValue::set(now),
                updated_at: ActiveValue::set(now),
            };
            row.insert(&self.dbc)
                .await
                .expect("insert episode_playlist");
        }
    }

    /// Link a podcast to a playlist so new episodes auto-add to it on poll.
    pub async fn seed_podcast_auto_playlist(&self, podcast_id: i32, playlist_id: i32) {
        use halogen_orm::podcast_auto_playlist::ActiveModel as PodcastAutoPlaylistActiveModel;

        let now = chrono::Utc::now();
        let row = PodcastAutoPlaylistActiveModel {
            podcast_id: ActiveValue::set(podcast_id),
            playlist_id: ActiveValue::set(playlist_id),
            add_to_start: ActiveValue::set(None),
            created_at: ActiveValue::set(now),
            updated_at: ActiveValue::set(now),
        };
        row.insert(&self.dbc)
            .await
            .expect("insert podcast_auto_playlist");
    }

    /// Set the per-user listen state for the given episodes (e.g. mark a subset `Finished` so the
    /// Played/Unplayed filter chips have a deterministic partition to test). State is per-user now — it's
    /// attributed to the admin/first user (the account the flows act as) in `user_episode_status`, which is
    /// what the server filters on (`filter[playback_status]`).
    pub async fn set_playback_status(
        &self,
        episode_ids: &[i32],
        status: halogen_wire::PlaybackStatus,
    ) {
        use halogen_orm::user_episode_status::{ActiveModel, Column, Entity as UesEntity};

        let Some(owner) = self.owner_user_id().await else {
            return;
        };
        let now = chrono::Utc::now();
        for &id in episode_ids {
            let existing = UesEntity::find()
                .filter(Column::UserId.eq(owner))
                .filter(Column::EpisodeId.eq(id))
                .one(&self.dbc)
                .await
                .expect("find user_episode_status");
            match existing {
                Some(row) => {
                    let mut am: ActiveModel = row.into();
                    am.playback_status = ActiveValue::set(status.clone());
                    am.updated_at = ActiveValue::set(now);
                    am.update(&self.dbc)
                        .await
                        .expect("update user_episode_status");
                }
                None => {
                    ActiveModel {
                        id: ActiveValue::NotSet,
                        user_id: ActiveValue::set(owner),
                        episode_id: ActiveValue::set(id),
                        playback_status: ActiveValue::set(status.clone()),
                        created_at: ActiveValue::set(now),
                        updated_at: ActiveValue::set(now),
                    }
                    .insert(&self.dbc)
                    .await
                    .expect("insert user_episode_status");
                }
            }
        }
    }

    /// Download the fixture clip through the real service and await completion, creating actual playable bytes and
    /// Downloaded state. Polling metadata alone does not enable play; the HTTP download endpoint is asynchronous.
    pub async fn download_on_server(&self, episode_id: i32) {
        use halogen_download as download;
        let opts = download::DownloadOptions {
            media_root: self.media_root.clone(),
            use_mock_download: true,
            tracker: std::sync::Arc::new(download::DownloadTracker::new()),
            retry: download::RetryPolicy::none(),
        };
        download::download_episode(&self.dbc, episode_id, &opts)
            .await
            .expect("server download");
    }

    /// The directory the server serves media from. Tests that stage a file for
    /// the audio/art endpoints must place it here — the handlers confine served
    /// paths to `media_root` (defense-in-depth against arbitrary-file reads).
    pub fn media_root(&self) -> &std::path::Path {
        &self.media_root
    }

    /// Stage `bytes` as episode `episode_id`'s downloaded audio *inside* `media_root` and flip the row to
    /// `Downloaded`, seeding the DB directly. This is the server-side equivalent of a real download (with
    /// caller-chosen bytes, so a test can assert exact/range content) — the write API no longer accepts
    /// `content_file_path`/`download_status`. Returns the staged path.
    pub async fn stage_downloaded_audio(
        &self,
        episode_id: i32,
        bytes: &[u8],
    ) -> std::path::PathBuf {
        use halogen_orm::episode::ActiveModel;
        use sea_orm::{ActiveModelTrait, ActiveValue};

        std::fs::create_dir_all(&self.media_root).expect("create media_root");
        let path = self.media_root.join(format!("{episode_id}.mp3"));
        std::fs::write(&path, bytes).expect("write staged audio");

        ActiveModel {
            id: ActiveValue::set(episode_id),
            download_status: ActiveValue::set(halogen_wire::DownloadStatus::Downloaded),
            content_file_path: ActiveValue::set(Some(path.display().to_string())),
            ..Default::default()
        }
        .update(&self.dbc)
        .await
        .expect("stage downloaded audio");
        path
    }

    /// Stage a real (decodable) PNG as podcast `podcast_id`'s cached artwork by writing it into
    /// `<media_root>/art/` and setting `art_file_path` — so `GET /podcasts/{id}/art` serves 200 with real bytes
    /// (and `/art/small` generates a thumbnail) instead of the 204 an art-less seed produces. Lets an art test
    /// assert real cached content, not just a key's presence.
    pub async fn seed_podcast_art(&self, podcast_id: i32) -> std::path::PathBuf {
        use halogen_orm::podcast::ActiveModel;
        use sea_orm::{ActiveModelTrait, ActiveValue};

        let dir = self.media_root.join("art");
        std::fs::create_dir_all(&dir).expect("create art dir");
        let path = dir.join(format!("podcast_{podcast_id}.png"));
        std::fs::write(&path, PNG_2X2).expect("write seeded art");

        ActiveModel {
            id: ActiveValue::set(podcast_id),
            art_file_path: ActiveValue::set(Some(path.display().to_string())),
            ..Default::default()
        }
        .update(&self.dbc)
        .await
        .expect("set podcast art_file_path");
        path
    }

    /// Id of the newest episode by `published_at` — the first row `/latest`
    /// shows (it sorts PublishedAt desc). Lets a play journey stage exactly the
    /// episode it will click first.
    pub async fn newest_episode_id(&self) -> i32 {
        use halogen_orm::episode::{Column, Entity as EpisodeEntity};
        use sea_orm::QueryOrder;

        EpisodeEntity::find()
            .order_by_desc(Column::PublishedAt)
            .one(&self.dbc)
            .await
            .expect("query newest episode")
            .expect("at least one episode exists")
            .id
    }

    /// Id of the first episode whose title contains `substr` (the one a search
    /// in the UI would surface). Panics if none match.
    pub async fn episode_id_by_title(&self, substr: &str) -> i32 {
        use halogen_orm::episode::{Column, Entity as EpisodeEntity};

        EpisodeEntity::find()
            .filter(Column::Title.contains(substr))
            .one(&self.dbc)
            .await
            .expect("query episode by title")
            .unwrap_or_else(|| panic!("no episode title contains {substr:?}"))
            .id
    }

    /// Force the `download_status` column on the given episodes (e.g. mark a
    /// subset `Downloaded` so the Downloaded filter chip has rows to match).
    pub async fn set_download_status(
        &self,
        episode_ids: &[i32],
        status: halogen_wire::DownloadStatus,
    ) {
        use halogen_orm::episode::{ActiveModel, Entity as EpisodeEntity};

        for &id in episode_ids {
            let model = EpisodeEntity::find_by_id(id)
                .one(&self.dbc)
                .await
                .expect("find episode")
                .expect("episode exists");
            let mut am: ActiveModel = model.into();
            am.download_status = ActiveValue::set(status.clone());
            am.update(&self.dbc).await.expect("update download_status");
        }
    }

    /// Seed a playback row per episode for `user_id` (alternating completed /
    /// in-progress) so the History list — which keeps only episodes that have a
    /// playback — has rows to render.
    pub async fn seed_playbacks(&self, user_id: i32, episode_ids: &[i32]) {
        use halogen_orm::playback::ActiveModel as PlaybackActiveModel;

        let now = chrono::Utc::now();
        for (i, &episode_id) in episode_ids.iter().enumerate() {
            let row = PlaybackActiveModel {
                user_id: ActiveValue::set(user_id),
                episode_id: ActiveValue::set(episode_id),
                cursor: ActiveValue::set(0),
                completed: ActiveValue::set(i % 2 == 0),
                created_at: ActiveValue::set(now),
                updated_at: ActiveValue::set(now),
                ..Default::default()
            };
            row.insert(&self.dbc).await.expect("insert playback");
        }
    }

    /// Seed chapter markers on an episode (read-only; normally written by sync).
    /// `chapters` is `(title, starts_at_secs)` in any order — they're stored as
    /// given so a test can also assert the handler returns them start-ordered.
    pub async fn seed_chapters(&self, episode_id: i32, chapters: &[(&str, i32)]) {
        use halogen_orm::episode_chapter::ActiveModel as ChapterActiveModel;

        let now = chrono::Utc::now();
        for &(title, starts_at_secs) in chapters {
            let row = ChapterActiveModel {
                episode_id: ActiveValue::set(episode_id),
                title: ActiveValue::set(title.to_string()),
                starts_at_secs: ActiveValue::set(starts_at_secs),
                created_at: ActiveValue::set(now),
                updated_at: ActiveValue::set(now),
                ..Default::default()
            };
            row.insert(&self.dbc).await.expect("insert chapter");
        }
    }
}
