use super::upload::bad_request;
use crate::db_error;
use halogen_orm::{
    episode, episode_chapter, episode_playlist, playback, playlist, podcast, podcast_auto_playlist,
    podcast_config, user, user_episode_status, user_podcast,
};
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, TransactionTrait};
use std::collections::HashMap;
use tracing::info;
pub(super) async fn merge_from(
    dbc: &DatabaseConnection,
    path: &std::path::Path,
) -> Result<DbImportSummaryData, ValidationErrors> {
    let src = halogen_migrations::get_dbc(&path.to_path_buf())
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
    halogen_queries::snapshot::migration_versions(conn)
        .await
        .map_err(|e| {
            bad_request(format!(
                "Failed to read the {what} database's schema version: {e}"
            ))
        })
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
    let summary = super::transaction::merge_in_txn(
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
