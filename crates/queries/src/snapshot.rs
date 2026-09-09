use halogen_orm::{
    episode, episode_download_error, podcast, podcast_sync_error, poll_job, poll_job_podcast, user,
};
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, Set, Statement};
use std::path::Path;

/// Create a consistent database snapshot. The caller must authorize whole-library export.
pub async fn create_snapshot(conn: &impl ConnectionTrait, destination: &Path) -> Result<(), DbErr> {
    if !destination.is_absolute() {
        return Err(DbErr::Custom(
            "Snapshot destination must be absolute".into(),
        ));
    }
    let path = destination
        .to_str()
        .filter(|p| !p.is_empty() && !p.contains('\0'))
        .ok_or_else(|| DbErr::Custom("Invalid snapshot destination".into()))?;
    conn.execute_raw(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "VACUUM INTO ?",
        [path.into()],
    ))
    .await?;
    Ok(())
}

/// Remove credentials, machine-local file state and operational history from a snapshot copy.
pub async fn scrub_snapshot(conn: &impl ConnectionTrait) -> Result<(), DbErr> {
    user::Entity::update_many()
        .set(user::ActiveModel {
            password_hash: Set(String::new()),
            ..Default::default()
        })
        .exec(conn)
        .await?;
    episode::Entity::update_many()
        .set(episode::ActiveModel {
            download_status: Set(halogen_wire::DownloadStatus::NotDownloaded),
            content_file_path: Set(None),
            download_size: Set(None),
            downloaded_at: Set(None),
            download_started_at: Set(None),
            download_attempts: Set(0),
            art_file_path: Set(None),
            ..Default::default()
        })
        .exec(conn)
        .await?;
    podcast::Entity::update_many()
        .set(podcast::ActiveModel {
            art_file_path: Set(None),
            ..Default::default()
        })
        .exec(conn)
        .await?;
    poll_job_podcast::Entity::delete_many().exec(conn).await?;
    poll_job::Entity::delete_many().exec(conn).await?;
    podcast_sync_error::Entity::delete_many().exec(conn).await?;
    episode_download_error::Entity::delete_many()
        .exec(conn)
        .await?;
    // The exported copy has no reusable change history; importing it logs changes in the target.
    conn.execute_raw(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "DELETE FROM sync_change",
        [],
    ))
    .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "UPDATE sync_epoch SET epoch = lower(hex(randomblob(16))), floor_sequence = COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'sync_change'), 0) WHERE id = 1", [],
    )).await?;
    conn.execute_raw(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "VACUUM",
        [],
    ))
    .await?;
    Ok(())
}

/// Read applied migration identities before an authorized database import.
pub async fn migration_versions(conn: &impl ConnectionTrait) -> Result<Vec<String>, DbErr> {
    conn.query_all_raw(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT version FROM seaql_migrations ORDER BY version",
        [],
    ))
    .await?
    .into_iter()
    .map(|row| row.try_get_by_index(0))
    .collect()
}
