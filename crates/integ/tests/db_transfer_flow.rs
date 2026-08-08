//! DB export/import journey — the server↔server / embedded↔server migration
//! and full-backup story.
//!
//! Export must be a WAL-safe, scrubbed snapshot: no password hashes, nothing
//! marked downloaded (and no file paths — media doesn't travel), no
//! operational history (poll jobs / error logs). Import must MERGE, never
//! replace: users match by username (the overlapping `admin` merges; `alice`
//! is created with a random password), podcasts/episodes/playlists land under
//! the mapped owner, and a re-import of the same file changes nothing.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(db_transfer_flow)'`

use std::io::Read;

use flate2::read::GzDecoder;
use halogen_api::{ApiClient, ApiError};
use halogen_integ::*;
use halogen_wire::{DownloadStatus, LoginData, PlaybackStoreData, PlaylistStoreData};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[tokio::test]
async fn db_export_import_journey() {
    // ── Source server A: admin + a second user + a real library ────────────
    let a = spawn().await;
    let a_admin = a.seed_admin().await;
    let a_client = api(&a, &a_admin.token);
    let (_a_podcast_id, a_episode_id) = subscribe_and_poll(&a, &a_client, "sed_podcast.xml").await;
    // A holds the file (Downloaded) — the export must strip that.
    a.download_on_server(a_episode_id).await;
    // Playback progress + a named playlist with the episode in it.
    a_client
        .upsert_playback(PlaybackStoreData {
            episode_id: a_episode_id,
            cursor: 123,
            completed: false,
        })
        .await
        .expect("playback");
    // The first playlist must be the default queue (server invariant), then a
    // named one — both should travel.
    a_client
        .create_playlist(PlaylistStoreData {
            name: "Queue".to_string(),
            description: None,
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .expect("queue");
    let playlist = a_client
        .create_playlist(PlaylistStoreData {
            name: "Road trip".to_string(),
            description: None,
            is_default: None,
            ..Default::default()
        })
        .await
        .expect("playlist");
    a_client
        .add_episode(playlist.id, a_episode_id, None)
        .await
        .expect("playlist add");
    let (alice_id, alice_password) = a.seed_user("alice").await;

    // ── Export: admin-only, gzipped SQLite, fully scrubbed ─────────────────
    let alice_token = a.jwt(&alice_id.to_string());
    let alice_client = api(&a, &alice_token);
    // The 403 body carries the standard errors envelope, so the client
    // surfaces it as a Validation error ("Admin privileges required").
    match alice_client.export_db().await {
        Err(ApiError::Validation(errors)) => {
            let text = format!("{errors:?}");
            assert!(
                text.contains("unauthorized"),
                "expected unauthorized: {text}"
            );
        }
        Err(ApiError::Server { status: 403, .. }) => {}
        other => panic!("non-admin export must be rejected, got {other:?}"),
    }

    let export = a_client.export_db().await.expect("export");
    assert_eq!(&export[..2], &[0x1f, 0x8b], "export is gzipped");
    let mut raw = Vec::new();
    GzDecoder::new(&export[..])
        .read_to_end(&mut raw)
        .expect("gunzip export");
    assert_eq!(&raw[..16], b"SQLite format 3\0", "export is a SQLite db");

    // Crack the snapshot open and verify the scrub directly.
    let inspect_dir =
        std::env::temp_dir().join(format!("halogen_db_export_inspect_{}", std::process::id()));
    std::fs::create_dir_all(&inspect_dir).expect("create inspect dir");
    let snap_path = inspect_dir.join("export.db");
    std::fs::write(&snap_path, &raw).expect("write snapshot");
    let snap = halogen_migrate::get_dbc(&snap_path)
        .await
        .expect("open snapshot");
    let users = halogen_orm::user::Entity::find()
        .all(&snap)
        .await
        .expect("snapshot users");
    assert!(users.len() >= 2);
    assert!(
        users.iter().all(|u| u.password_hash.is_empty()),
        "password hashes must be stripped"
    );
    let episodes = halogen_orm::episode::Entity::find()
        .all(&snap)
        .await
        .expect("snapshot episodes");
    assert!(!episodes.is_empty());
    assert!(
        episodes.iter().all(|e| {
            e.download_status == DownloadStatus::NotDownloaded
                && e.content_file_path.is_none()
                && e.downloaded_at.is_none()
                && e.download_attempts == 0
        }),
        "nothing in an export may claim downloaded bytes"
    );
    let jobs = halogen_orm::poll_job::Entity::find()
        .all(&snap)
        .await
        .expect("snapshot poll jobs");
    assert!(jobs.is_empty(), "operational history must not travel");
    let _ = snap.close().await;
    let _ = std::fs::remove_dir_all(&inspect_dir);

    // ── Target server B: its own admin (SAME username) already exists ──────
    let b = spawn().await;
    let b_admin = b.seed_admin().await;
    let b_client = api(&b, &b_admin.token);

    let summary = b_client.import_db(export.clone()).await.expect("import");
    assert_eq!(summary.users_merged, 1, "admin merged by username");
    assert_eq!(summary.users_created, 1, "alice created");
    assert_eq!(summary.created_usernames, vec!["alice".to_string()]);
    assert_eq!(summary.podcasts_created, 1);
    assert_eq!(summary.podcasts_merged, 0);
    assert!(summary.episodes_created >= 1);
    assert!(summary.playbacks_upserted >= 1);
    // "Road trip" + A-admin's default queue land as new playlists on B.
    assert!(summary.playlists_created >= 2);
    assert!(summary.playlist_links_created >= 1);
    assert_eq!(summary.subscriptions_created, 1);

    // Merged library belongs to B's admin; nothing claims local bytes.
    let b_podcasts = halogen_orm::podcast::Entity::find()
        .all(&b.dbc)
        .await
        .expect("b podcasts");
    assert_eq!(b_podcasts.len(), 1);
    assert_eq!(
        b_podcasts[0].owner_id, b_admin.id,
        "imported podcast attributed to the MERGED (target) admin"
    );
    let b_episodes = halogen_orm::episode::Entity::find()
        .all(&b.dbc)
        .await
        .expect("b episodes");
    assert!(b_episodes.iter().all(|e| {
        e.download_status == DownloadStatus::NotDownloaded && e.content_file_path.is_none()
    }));

    // Alice exists with a fresh RANDOM password: her A-side password no longer
    // works, and the stored hash is non-empty (never the stripped sentinel).
    let b_alice = halogen_orm::user::Entity::find()
        .filter(halogen_orm::user::Column::Username.eq("alice"))
        .one(&b.dbc)
        .await
        .expect("query alice")
        .expect("alice exists on B");
    assert!(!b_alice.password_hash.is_empty());
    let fresh_login = ApiClient::new(b.base_url.parse().expect("b url"));
    match fresh_login
        .login(LoginData {
            username: "alice".to_string(),
            password: alice_password.clone(),
        })
        .await
    {
        Err(ApiError::Server { status: 401, .. }) => {}
        other => panic!("created user must have a NEW random password, got {other:?}"),
    }

    // B's admin keeps their own credentials (merge never touches the hash).
    fresh_login
        .login(LoginData {
            username: b_admin.username.clone(),
            password: b_admin.password.clone(),
        })
        .await
        .expect("target admin login still works after import");

    // ── Idempotency: importing the same file again changes nothing ─────────
    let again = b_client.import_db(export).await.expect("re-import");
    assert_eq!(again.users_created, 0);
    assert_eq!(again.users_merged, 2);
    assert_eq!(again.podcasts_created, 0);
    assert_eq!(again.podcasts_merged, 1);
    assert_eq!(again.episodes_created, 0);
    assert_eq!(again.subscriptions_created, 0);
    assert_eq!(again.playlists_created, 0);
    assert_eq!(again.playlist_links_created, 0);

    // Garbage in → a clear client error (409 conflict envelope), DB untouched.
    match b_client.import_db(b"not a database".to_vec()).await {
        Err(ApiError::Validation(_)) => {}
        Err(ApiError::Server { status, message }) => {
            assert_eq!(status, 409, "conflict for a non-sqlite payload");
            assert!(message.contains("Not a SQLite database"), "got: {message}");
        }
        other => panic!("garbage import must be a client error, got {other:?}"),
    }
}
