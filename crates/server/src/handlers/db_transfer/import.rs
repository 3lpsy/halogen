//! Merge an uploaded DB export into the live database.
//!
//! NEVER drop-and-replace: the target has at least its own admin, and may have
//! a full library. Identity per entity:
//! - **users** by lowercased username (matching users merge; new ones are
//!   created — with a fresh random password when the export's hash was
//!   stripped, reported in `created_usernames` so an embedded host can align
//!   its silent-login secrets);
//! - **podcasts** by `(owner, feed_url)` within a mapped user;
//! - **episodes** by `guid` (falling back to `content_url`) within a mapped
//!   podcast;
//! - **playlists** by the per-user default flag, else `(user, name)`;
//! - playbacks / episode statuses upsert keeping the newer `updated_at`;
//!   pivots (subscriptions, playlist membership, auto-playlists) insert-if-
//!   missing, memberships appended after the target's existing positions.
//!
//! Operational tables (poll jobs, sync/download error logs) are never
//! imported, and download/file fields are re-reset on insert as defense in
//! depth (the export already scrubs them — media files don't travel).
//!
//! The whole merge runs in one transaction on the target: an import either
//! fully lands or leaves the DB untouched.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, NotSet, QueryFilter, Set, Statement, TransactionTrait,
};
use tracing::{info, warn};

use crate::handlers::db_error;
use crate::routers::db_transfer::DB_IMPORT_MAX_DECOMPRESSED_BYTES;
use halogen_orm::{
    episode, episode_chapter, episode_playlist, playback, playlist, podcast, podcast_auto_playlist,
    podcast_config, user, user_episode_status, user_podcast,
};
use halogen_utils::constants::{
    VALIDATION_CONFLICT_CODE, VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE,
    VALIDATION_REQUEST_FIELD,
};
use halogen_utils::verrors;

const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

fn bad_request(msg: impl Into<String>) -> ValidationErrors {
    verrors(
        VALIDATION_REQUEST_FIELD,
        VALIDATION_CONFLICT_CODE,
        msg.into(),
    )
}

/// Accept a gzipped or raw SQLite export and merge it into `dbc`.
pub async fn handle(
    dbc: &DatabaseConnection,
    body: &[u8],
) -> Result<DbImportSummaryData, ValidationErrors> {
    // Decompression + temp-file staging are blocking (CPU + file I/O, up to
    // the 1 GiB decompressed cap) — off the reactor, like the export side.
    let body = body.to_vec();
    let (path, dir) = tokio::task::spawn_blocking(move || stage_upload(&body))
        .await
        .map_err(|e| {
            verrors(
                VALIDATION_REQUEST_FIELD,
                VALIDATION_PANIC_CODE,
                format!("Import staging task failed: {e}"),
            )
        })??;
    let result = merge_from(dbc, &path).await;
    let _ = tokio::task::spawn_blocking(move || fs::remove_dir_all(&dir)).await;
    result
}

/// Gunzip (size-capped) or take the payload raw, then land it in a fresh temp
/// dir so SQLite can open it. Returns `(db file, temp dir to clean up)`.
fn stage_upload(body: &[u8]) -> Result<(PathBuf, PathBuf), ValidationErrors> {
    // Gunzip when the payload carries the gzip magic; otherwise take it raw.
    let raw = if body.len() >= 2 && body[..2] == GZIP_MAGIC {
        let mut out = Vec::new();
        // `take(cap + 1)`: a decompression bomb stops at the cap instead of
        // exhausting memory; landing above the cap means the payload overran it.
        let cap = DB_IMPORT_MAX_DECOMPRESSED_BYTES as u64;
        let mut decoder = GzDecoder::new(body).take(cap + 1);
        decoder
            .read_to_end(&mut out)
            .map_err(|e| bad_request(format!("Invalid gzip payload: {e}")))?;
        if out.len() as u64 > cap {
            return Err(bad_request(format!(
                "Decompressed import exceeds the {} MiB limit",
                DB_IMPORT_MAX_DECOMPRESSED_BYTES / (1024 * 1024)
            )));
        }
        out
    } else {
        body.to_vec()
    };
    if raw.len() < SQLITE_MAGIC.len() || &raw[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
        return Err(bad_request(
            "Not a SQLite database (expected a Halogen DB export, .db or .db.gz)",
        ));
    }

    // Land the upload in a temp file so SQLite can open it.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "halogen-db-import-{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir)
        .map_err(|e| bad_request(format!("Failed to stage the import: {e}")))?;
    let path = dir.join("import.db");
    if let Err(e) = fs::write(&path, &raw) {
        let _ = fs::remove_dir_all(&dir);
        return Err(bad_request(format!("Failed to stage the import: {e}")));
    }
    Ok((path, dir))
}

async fn merge_from(
    dbc: &DatabaseConnection,
    path: &std::path::Path,
) -> Result<DbImportSummaryData, ValidationErrors> {
    let src = halogen_migrate::get_dbc(&path.to_path_buf())
        .await
        .map_err(|e| bad_request(format!("Failed to open the uploaded database: {e}")))?;

    let outcome = merge(dbc, &src).await;
    let _ = src.close().await;
    outcome
}

/// The applied-migration set of a DB, for the schema guard.
async fn migration_versions(
    conn: &impl ConnectionTrait,
    what: &str,
) -> Result<Vec<String>, ValidationErrors> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            conn.get_database_backend(),
            "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
        ))
        .await
        .map_err(|e| {
            bad_request(format!(
                "Failed to read the {what} database's schema version: {e}"
            ))
        })?;
    rows.into_iter()
        .map(|r| {
            r.try_get_by_index::<String>(0).map_err(|e| {
                verrors(
                    VALIDATION_DATABASE_FIELD,
                    VALIDATION_CONFLICT_CODE,
                    format!("Failed to decode the {what} schema version: {e}"),
                )
            })
        })
        .collect()
}

async fn merge(
    dbc: &DatabaseConnection,
    src: &DatabaseConnection,
) -> Result<DbImportSummaryData, ValidationErrors> {
    // Schema guard: the import must carry exactly the target's migration set —
    // a newer export would reference columns this server doesn't have (and an
    // older one would miss ours). Clear error either way.
    let src_versions = migration_versions(src, "uploaded").await?;
    let dst_versions = migration_versions(dbc, "live").await?;
    if src_versions != dst_versions {
        return Err(bad_request(format!(
            "Schema mismatch: the export has {} applied migrations, this server has {} — \
             update both servers to the same version and re-export",
            src_versions.len(),
            dst_versions.len()
        )));
    }

    // Read the entire source (metadata-only DB — small by construction).
    let src_users = user::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported users"))?;
    let src_podcasts = podcast::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported podcasts"))?;
    let src_configs: HashMap<i32, podcast_config::Model> = podcast_config::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported podcast configs"))?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    let src_episodes = episode::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported episodes"))?;
    let src_chapters = episode_chapter::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported chapters"))?;
    let src_subscriptions = user_podcast::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported subscriptions"))?;
    let src_playbacks = playback::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported playbacks"))?;
    let src_statuses = user_episode_status::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported episode statuses"))?;
    let mut src_playlists = playlist::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported playlists"))?;
    let mut src_links = episode_playlist::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported playlist memberships"))?;
    let src_auto = podcast_auto_playlist::Entity::find()
        .all(src)
        .await
        .map_err(db_error("reading imported auto-playlists"))?;

    // Stable orders so appended positions preserve the source's relative order.
    src_playlists.sort_by_key(|p| (p.user_id, p.position, p.id));
    src_links.sort_by_key(|l| (l.playlist_id, l.position, l.episode_id));

    let txn = dbc
        .begin()
        .await
        .map_err(db_error("starting the import transaction"))?;
    let summary = merge_in_txn(
        &txn,
        src_users,
        src_podcasts,
        src_configs,
        src_episodes,
        src_chapters,
        src_subscriptions,
        src_playbacks,
        src_statuses,
        src_playlists,
        src_links,
        src_auto,
    )
    .await?;
    txn.commit()
        .await
        .map_err(db_error("committing the import"))?;

    info!(
        users_created = summary.users_created,
        users_merged = summary.users_merged,
        podcasts_created = summary.podcasts_created,
        episodes_created = summary.episodes_created,
        "DB import merged"
    );
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
async fn merge_in_txn(
    txn: &DatabaseTransaction,
    src_users: Vec<user::Model>,
    src_podcasts: Vec<podcast::Model>,
    src_configs: HashMap<i32, podcast_config::Model>,
    src_episodes: Vec<episode::Model>,
    src_chapters: Vec<episode_chapter::Model>,
    src_subscriptions: Vec<user_podcast::Model>,
    src_playbacks: Vec<playback::Model>,
    src_statuses: Vec<user_episode_status::Model>,
    src_playlists: Vec<playlist::Model>,
    src_links: Vec<episode_playlist::Model>,
    src_auto: Vec<podcast_auto_playlist::Model>,
) -> Result<DbImportSummaryData, ValidationErrors> {
    let mut summary = DbImportSummaryData::default();

    // ── users: merge by lowercased username ────────────────────────────────
    let mut user_map: HashMap<i32, i32> = HashMap::new();
    for su in &src_users {
        let uname = su.username.to_lowercase();
        let existing = user::Entity::find()
            .filter(user::Column::Username.eq(uname.clone()))
            .one(txn)
            .await
            .map_err(db_error("matching an imported user"))?;
        match existing {
            Some(t) => {
                user_map.insert(su.id, t.id);
                summary.users_merged += 1;
            }
            None => {
                // Stripped exports carry empty hashes — provision a random
                // password (never returned; an embedded host re-keys its own
                // silent-login secrets from `created_usernames`).
                let hash = if su.password_hash.is_empty() {
                    bcrypt::hash(halogen_orm::user::generate_password(), bcrypt::DEFAULT_COST)
                        .map_err(|e| bad_request(format!("Failed to hash a password: {e}")))?
                } else {
                    su.password_hash.clone()
                };
                // Explicit id: the admin sentinel at i32::MAX breaks sqlite's
                // implicit successor (see `next_available_id`).
                let id = halogen_orm::user::next_available_id(txn)
                    .await
                    .map_err(db_error("allocating an imported user id"))?;
                let created = user::ActiveModel {
                    id: Set(id),
                    username: Set(uname.clone()),
                    password_hash: Set(hash),
                    is_admin: Set(su.is_admin),
                    created_at: Set(su.created_at),
                    updated_at: Set(su.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported user"))?;
                user_map.insert(su.id, created.id);
                summary.users_created += 1;
                summary.created_usernames.push(uname);
            }
        }
    }

    // ── podcasts (+ per-podcast config) by (owner, feed_url) ───────────────
    let mut podcast_map: HashMap<i32, i32> = HashMap::new();
    // Podcasts whose target row already existed — their episodes need matching
    // instead of blind insertion.
    let mut merged_podcasts: Vec<i32> = Vec::new();
    for sp in &src_podcasts {
        let Some(&owner) = user_map.get(&sp.owner_id) else {
            warn!(podcast = sp.id, "Skipping podcast with unmapped owner");
            continue;
        };
        // `feed_url` is GLOBALLY unique — there is one shared podcast row per feed
        // (see `podcast_store`). Match on it ALONE, not `(owner, feed_url)`: if the
        // target already has this feed under a DIFFERENT owner, map to that shared
        // row (the subscription phase attaches the imported user; the owner stays
        // the original adder). The old `(owner, feed_url)` match missed such a row
        // and then tried to INSERT a duplicate `feed_url`, firing the UNIQUE index
        // and rolling back the ENTIRE import.
        let existing = podcast::Entity::find()
            .filter(podcast::Column::FeedUrl.eq(sp.feed_url.clone()))
            .one(txn)
            .await
            .map_err(db_error("matching an imported podcast"))?;
        match existing {
            Some(t) => {
                podcast_map.insert(sp.id, t.id);
                merged_podcasts.push(sp.id);
                summary.podcasts_merged += 1;
            }
            None => {
                // Clone the per-podcast config first (fresh id), if any.
                let config_id = match sp.podcast_config_id.and_then(|id| src_configs.get(&id)) {
                    Some(cfg) => Some(
                        podcast_config::ActiveModel {
                            id: NotSet,
                            poll_interval_seconds: Set(cfg.poll_interval_seconds),
                            max_episodes: Set(cfg.max_episodes),
                            max_concurrent_downloads: Set(cfg.max_concurrent_downloads),
                            auto_download_enabled: Set(cfg.auto_download_enabled),
                            created_at: Set(cfg.created_at),
                            updated_at: Set(cfg.updated_at),
                        }
                        .insert(txn)
                        .await
                        .map_err(db_error("creating an imported podcast config"))?
                        .id,
                    ),
                    None => None,
                };
                let created = podcast::ActiveModel {
                    id: NotSet,
                    title: Set(sp.title.clone()),
                    description: Set(sp.description.clone()),
                    feed_url: Set(sp.feed_url.clone()),
                    art_url: Set(sp.art_url.clone()),
                    // Machine-local: the art cache file doesn't travel.
                    art_file_path: Set(None),
                    author: Set(sp.author.clone()),
                    etag: Set(sp.etag.clone()),
                    last_modified: Set(sp.last_modified.clone()),
                    polled_at: Set(sp.polled_at),
                    podcast_config_id: Set(config_id),
                    owner_id: Set(owner),
                    feed_url_redirects: Set(sp.feed_url_redirects.clone()),
                    created_at: Set(sp.created_at),
                    updated_at: Set(sp.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported podcast"))?;
                podcast_map.insert(sp.id, created.id);
                summary.podcasts_created += 1;
            }
        }
    }

    // ── subscriptions (user_podcast) insert-if-missing ─────────────────────
    // Preload existing (user, podcast) pairs once — batch-match like episodes.
    let mut existing_subs: std::collections::HashSet<(i32, i32)> = user_podcast::Entity::find()
        .all(txn)
        .await
        .map_err(db_error("reading target subscriptions"))?
        .into_iter()
        .map(|s| (s.user_id, s.podcast_id))
        .collect();
    for sub in &src_subscriptions {
        let (Some(&uid), Some(&pid)) =
            (user_map.get(&sub.user_id), podcast_map.get(&sub.podcast_id))
        else {
            continue;
        };
        if existing_subs.insert((uid, pid)) {
            user_podcast::ActiveModel {
                user_id: Set(uid),
                podcast_id: Set(pid),
                created_at: Set(sub.created_at),
                updated_at: Set(sub.updated_at),
            }
            .insert(txn)
            .await
            .map_err(db_error("creating an imported subscription"))?;
            summary.subscriptions_created += 1;
        }
    }

    // ── episodes by guid (fallback content_url) within a podcast ───────────
    let mut episode_map: HashMap<i32, i32> = HashMap::new();
    // Per merged target podcast: existing episodes keyed by guid and by
    // content_url, preloaded once.
    let mut target_eps: HashMap<i32, (HashMap<String, i32>, HashMap<String, i32>)> = HashMap::new();
    for src_pid in &merged_podcasts {
        let tgt_pid = podcast_map[src_pid];
        let existing = episode::Entity::find()
            .filter(episode::Column::PodcastId.eq(tgt_pid))
            .all(txn)
            .await
            .map_err(db_error("reading target episodes for matching"))?;
        let mut by_guid = HashMap::new();
        let mut by_url = HashMap::new();
        for e in existing {
            if let Some(g) = &e.guid {
                by_guid.insert(g.clone(), e.id);
            }
            by_url.insert(e.content_url.clone(), e.id);
        }
        target_eps.insert(tgt_pid, (by_guid, by_url));
    }
    for se in &src_episodes {
        let Some(&tgt_pid) = podcast_map.get(&se.podcast_id) else {
            continue;
        };
        let matched = target_eps.get(&tgt_pid).and_then(|(by_guid, by_url)| {
            se.guid
                .as_ref()
                .and_then(|g| by_guid.get(g))
                .or_else(|| by_url.get(&se.content_url))
                .copied()
        });
        match matched {
            Some(tid) => {
                episode_map.insert(se.id, tid);
                summary.episodes_merged += 1;
            }
            None => {
                let created = episode::ActiveModel {
                    id: NotSet,
                    podcast_id: Set(tgt_pid),
                    title: Set(se.title.clone()),
                    description: Set(se.description.clone()),
                    content_url: Set(se.content_url.clone()),
                    guid: Set(se.guid.clone()),
                    art_url: Set(se.art_url.clone()),
                    published_at: Set(se.published_at),
                    // Defense in depth (exports are already scrubbed): nothing
                    // imported may claim bytes on this machine's disk.
                    downloaded_at: Set(None),
                    content_file_path: Set(None),
                    download_size: Set(None),
                    art_file_path: Set(None),
                    download_status: Set(halogen_wire::DownloadStatus::NotDownloaded),
                    download_started_at: Set(None),
                    download_attempts: Set(0),
                    duration_secs: Set(se.duration_secs),
                    created_at: Set(se.created_at),
                    updated_at: Set(se.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported episode"))?;
                episode_map.insert(se.id, created.id);
                summary.episodes_created += 1;
                if let Some((by_guid, by_url)) = target_eps.get_mut(&tgt_pid) {
                    if let Some(g) = &se.guid {
                        by_guid.insert(g.clone(), created.id);
                    }
                    by_url.insert(se.content_url.clone(), created.id);
                }
                // Chapters ride only with episodes we created (matched ones
                // keep the target's).
                for ch in src_chapters.iter().filter(|c| c.episode_id == se.id) {
                    episode_chapter::ActiveModel {
                        id: NotSet,
                        episode_id: Set(created.id),
                        title: Set(ch.title.clone()),
                        starts_at_secs: Set(ch.starts_at_secs),
                        created_at: Set(ch.created_at),
                        updated_at: Set(ch.updated_at),
                    }
                    .insert(txn)
                    .await
                    .map_err(db_error("creating an imported chapter"))?;
                    summary.chapters_created += 1;
                }
            }
        }
    }

    // ── playbacks upsert (newer updated_at wins) ────────────────────────────
    // Preload existing playbacks once, keyed by (user, episode) — history is
    // the biggest table an import carries; per-row point queries would make
    // the merge O(rows) round-trips inside the transaction.
    let mut target_playbacks: HashMap<(i32, i32), playback::Model> = playback::Entity::find()
        .all(txn)
        .await
        .map_err(db_error("reading target playbacks"))?
        .into_iter()
        .map(|p| ((p.user_id, p.episode_id), p))
        .collect();
    for pb in &src_playbacks {
        let (Some(&uid), Some(&eid)) = (user_map.get(&pb.user_id), episode_map.get(&pb.episode_id))
        else {
            continue;
        };
        let existing = target_playbacks.get(&(uid, eid)).cloned();
        match existing {
            Some(t) if t.updated_at >= pb.updated_at => {}
            Some(t) => {
                let mut am: playback::ActiveModel = t.into();
                am.cursor = Set(pb.cursor);
                am.completed = Set(pb.completed);
                am.updated_at = Set(pb.updated_at);
                let updated = am
                    .update(txn)
                    .await
                    .map_err(db_error("updating an imported playback"))?;
                target_playbacks.insert((uid, eid), updated);
                summary.playbacks_upserted += 1;
            }
            None => {
                let created = playback::ActiveModel {
                    id: NotSet,
                    user_id: Set(uid),
                    episode_id: Set(eid),
                    cursor: Set(pb.cursor),
                    completed: Set(pb.completed),
                    created_at: Set(pb.created_at),
                    updated_at: Set(pb.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported playback"))?;
                target_playbacks.insert((uid, eid), created);
                summary.playbacks_upserted += 1;
            }
        }
    }

    // ── per-user episode statuses upsert (newer updated_at wins) ───────────
    // Same preload treatment as playbacks (the other per-user history table).
    let mut target_statuses: HashMap<(i32, i32), user_episode_status::Model> =
        user_episode_status::Entity::find()
            .all(txn)
            .await
            .map_err(db_error("reading target episode statuses"))?
            .into_iter()
            .map(|s| ((s.user_id, s.episode_id), s))
            .collect();
    for st in &src_statuses {
        let (Some(&uid), Some(&eid)) = (user_map.get(&st.user_id), episode_map.get(&st.episode_id))
        else {
            continue;
        };
        let existing = target_statuses.get(&(uid, eid)).cloned();
        match existing {
            Some(t) if t.updated_at >= st.updated_at => {}
            Some(t) => {
                let mut am: user_episode_status::ActiveModel = t.into();
                am.playback_status = Set(st.playback_status.clone());
                am.updated_at = Set(st.updated_at);
                let updated = am
                    .update(txn)
                    .await
                    .map_err(db_error("updating an imported episode status"))?;
                target_statuses.insert((uid, eid), updated);
                summary.statuses_upserted += 1;
            }
            None => {
                let created = user_episode_status::ActiveModel {
                    id: NotSet,
                    user_id: Set(uid),
                    episode_id: Set(eid),
                    playback_status: Set(st.playback_status.clone()),
                    created_at: Set(st.created_at),
                    updated_at: Set(st.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported episode status"))?;
                target_statuses.insert((uid, eid), created);
                summary.statuses_upserted += 1;
            }
        }
    }

    // ── playlists: default merges to default; others by (user, name) ───────
    let mut playlist_map: HashMap<i32, i32> = HashMap::new();
    // Next free per-user playlist position, computed lazily.
    let mut next_position: HashMap<i32, i32> = HashMap::new();
    for pl in &src_playlists {
        let Some(&uid) = user_map.get(&pl.user_id) else {
            continue;
        };
        let existing = if pl.is_default {
            playlist::Entity::find()
                .filter(playlist::Column::UserId.eq(uid))
                .filter(playlist::Column::IsDefault.eq(true))
                .one(txn)
                .await
                .map_err(db_error("matching an imported default playlist"))?
        } else {
            playlist::Entity::find()
                .filter(playlist::Column::UserId.eq(uid))
                .filter(playlist::Column::IsDefault.eq(false))
                .filter(playlist::Column::Name.eq(pl.name.clone()))
                .one(txn)
                .await
                .map_err(db_error("matching an imported playlist"))?
        };
        match existing {
            Some(t) => {
                playlist_map.insert(pl.id, t.id);
                summary.playlists_merged += 1;
            }
            None => {
                let pos = match next_position.get(&uid) {
                    Some(p) => *p,
                    None => {
                        let max = playlist::Entity::find()
                            .filter(playlist::Column::UserId.eq(uid))
                            .all(txn)
                            .await
                            .map_err(db_error("reading target playlists"))?
                            .into_iter()
                            .map(|p| p.position)
                            .max()
                            .unwrap_or(-1);
                        max + 1
                    }
                };
                next_position.insert(uid, pos + 1);
                let created = playlist::ActiveModel {
                    id: NotSet,
                    name: Set(pl.name.clone()),
                    description: Set(pl.description.clone()),
                    user_id: Set(uid),
                    // A brand-new user keeps their imported default; a user who
                    // already had one can't grow a second (matched above).
                    is_default: Set(pl.is_default),
                    position: Set(pos),
                    on_remove_delete_file_server: Set(pl.on_remove_delete_file_server),
                    on_remove_delete_file_client: Set(pl.on_remove_delete_file_client),
                    created_at: Set(pl.created_at),
                    updated_at: Set(pl.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported playlist"))?;
                playlist_map.insert(pl.id, created.id);
                summary.playlists_created += 1;
            }
        }
    }

    // ── playlist memberships: append after the target's tail ───────────────
    // Per target playlist: (existing episode ids, next free position).
    let mut membership: HashMap<i32, (std::collections::HashSet<i32>, i32)> = HashMap::new();
    for link in &src_links {
        let (Some(&plid), Some(&eid)) = (
            playlist_map.get(&link.playlist_id),
            episode_map.get(&link.episode_id),
        ) else {
            continue;
        };
        if !membership.contains_key(&plid) {
            let existing = episode_playlist::Entity::find()
                .filter(episode_playlist::Column::PlaylistId.eq(plid))
                .all(txn)
                .await
                .map_err(db_error("reading target playlist membership"))?;
            let next = existing.iter().map(|l| l.position).max().unwrap_or(-1) + 1;
            let ids = existing.into_iter().map(|l| l.episode_id).collect();
            membership.insert(plid, (ids, next));
        }
        let entry = membership.get_mut(&plid).expect("preloaded above");
        if entry.0.contains(&eid) {
            continue;
        }
        episode_playlist::ActiveModel {
            episode_id: Set(eid),
            playlist_id: Set(plid),
            position: Set(entry.1),
            created_at: Set(link.created_at),
            updated_at: Set(link.updated_at),
        }
        .insert(txn)
        .await
        .map_err(db_error("creating an imported playlist membership"))?;
        entry.0.insert(eid);
        entry.1 += 1;
        summary.playlist_links_created += 1;
    }

    // ── auto-playlist wiring insert-if-missing ──────────────────────────────
    for ap in &src_auto {
        let (Some(&pid), Some(&plid)) = (
            podcast_map.get(&ap.podcast_id),
            playlist_map.get(&ap.playlist_id),
        ) else {
            continue;
        };
        let exists = podcast_auto_playlist::Entity::find()
            .filter(podcast_auto_playlist::Column::PodcastId.eq(pid))
            .filter(podcast_auto_playlist::Column::PlaylistId.eq(plid))
            .one(txn)
            .await
            .map_err(db_error("matching an imported auto-playlist"))?;
        if exists.is_none() {
            podcast_auto_playlist::ActiveModel {
                podcast_id: Set(pid),
                playlist_id: Set(plid),
                add_to_start: Set(ap.add_to_start),
                created_at: Set(ap.created_at),
                updated_at: Set(ap.updated_at),
            }
            .insert(txn)
            .await
            .map_err(db_error("creating an imported auto-playlist"))?;
            summary.auto_playlists_created += 1;
        }
    }

    Ok(summary)
}
