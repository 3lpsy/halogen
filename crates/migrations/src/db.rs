use crate::migrations::Migrator;
use anyhow::{Result, anyhow};
use halogen_utils::{ensure_parent_dir, touch};
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sea_orm::{ConnectOptions, Database, DatabaseConnection, SqlxSqliteConnector};
use sea_orm_migration::MigratorTrait;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, info};

pub async fn migrate(db_path: &PathBuf) -> Result<()> {
    let db = get_dbc(db_path).await?;
    Migrator::up(&db, None).await?;
    info!("Database migrated");
    Ok(())
}

pub async fn rollback(db_path: &PathBuf) -> Result<()> {
    let db = get_dbc(db_path).await?;
    Migrator::down(&db, Some(1)).await?;
    info!("Database rollbacked");
    Ok(())
}

pub async fn get_dbc(db_path: &PathBuf) -> Result<DatabaseConnection> {
    // TODO: Okay, the parent directory might not exist. Probably just create
    touch(db_path)?;
    let db_url = get_db_url(db_path);
    debug!("Connecting: {}", db_url);
    let options = ConnectOptions::new(db_url);

    Ok(Database::connect(options).await?)
}

/// Set the SQLite journal explicitly; sqlx otherwise inherits the file's mode because switching needs an exclusive lock.
/// WAL supports concurrent readers/pools and creates -wal/-shm sidecars. Delete actively converts sticky WAL back
/// to rollback mode for --db-no-wal, including network filesystems where shared-memory WAL is unsafe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalMode {
    Wal,
    Delete,
}

/// Open with an explicit [`JournalMode`] + a 5s busy_timeout.
pub async fn get_dbc_with(db_path: &PathBuf, mode: JournalMode) -> Result<DatabaseConnection> {
    touch(db_path)?;
    let db_url = get_db_url(db_path);
    debug!("Connecting ({mode:?} journal): {db_url}");
    let journal = match mode {
        JournalMode::Wal => SqliteJournalMode::Wal,
        JournalMode::Delete => SqliteJournalMode::Delete,
    };
    let options = SqliteConnectOptions::from_str(&db_url)
        .map_err(|e| anyhow!("Invalid sqlite url {db_url}: {e}"))?
        .journal_mode(journal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .map_err(|e| anyhow!("Failed to connect to {db_url}: {e}"))?;
    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

/// [`get_dbc_with`] pinned to WAL — the profile the embedded (in-process)
/// server always uses (its restart overlap depends on it).
pub async fn get_dbc_wal(db_path: &PathBuf) -> Result<DatabaseConnection> {
    get_dbc_with(db_path, JournalMode::Wal).await
}

pub async fn connect_and_migrate(
    db_path: &PathBuf,
    run_migrate: bool,
) -> Result<DatabaseConnection> {
    connect_and_migrate_inner(db_path, run_migrate, None).await
}

/// [`connect_and_migrate`] with an explicit [`JournalMode`] — what the server
/// binary uses (`Wal` unless `--db-no-wal` pins `Delete`).
pub async fn connect_and_migrate_with(
    db_path: &PathBuf,
    run_migrate: bool,
    mode: JournalMode,
) -> Result<DatabaseConnection> {
    connect_and_migrate_inner(db_path, run_migrate, Some(mode)).await
}

/// [`connect_and_migrate_with`] pinned to WAL for concurrent local readers and writers.
pub async fn connect_and_migrate_wal(
    db_path: &PathBuf,
    run_migrate: bool,
) -> Result<DatabaseConnection> {
    connect_and_migrate_inner(db_path, run_migrate, Some(JournalMode::Wal)).await
}

async fn connect_and_migrate_inner(
    db_path: &PathBuf,
    run_migrate: bool,
    mode: Option<JournalMode>,
) -> Result<DatabaseConnection> {
    ensure_parent_dir(db_path)?;

    if !db_path.exists() {
        fs::File::create(db_path)
            .map_err(|e| anyhow!("Failed to create db file {:?}: {}", db_path, e))?;
    }

    info!(
        "Attempting to connect to database at: {}",
        db_path.display()
    );

    let dbc = match mode {
        Some(mode) => get_dbc_with(db_path, mode).await?,
        // Legacy implicit open (no journal pragma): inherits whatever the
        // file already runs. Kept for tooling/tests; the server + embedded
        // paths always pass an explicit mode.
        None => get_dbc(db_path).await?,
    };

    info!("Connected to database at {}", db_path.display());

    if run_migrate {
        info!("Running migrations for database at {}", db_path.display());
        Migrator::up(&dbc, None)
            .await
            .map_err(|e| anyhow!("Failed to run migrations at {}: {}", db_path.display(), e))?;
    }

    Ok(dbc)
}

pub fn get_db_url(db_path: impl AsRef<Path>) -> String {
    format!("sqlite://{}", db_path.as_ref().display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Statement};

    async fn pragmas(dbc: &DatabaseConnection) -> (String, i64) {
        let backend = dbc.get_database_backend();
        let journal = dbc
            .query_one_raw(Statement::from_string(backend, "PRAGMA journal_mode;"))
            .await
            .expect("query journal_mode")
            .expect("journal_mode row");
        let mode: String = journal.try_get_by_index(0).expect("journal_mode value");
        let busy = dbc
            .query_one_raw(Statement::from_string(backend, "PRAGMA busy_timeout;"))
            .await
            .expect("query busy_timeout")
            .expect("busy_timeout row");
        let timeout_ms: i64 = busy.try_get_by_index(0).expect("busy_timeout value");
        (mode.to_lowercase(), timeout_ms)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("halogen_migrations_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// WAL permits overlapping readers/pools; a nonzero busy_timeout lets writers wait for locks.
    #[tokio::test]
    async fn wal_profile_sets_wal_and_busy_timeout() {
        let dir = temp_dir("wal_profile");
        let dbc = connect_and_migrate_wal(&dir.join("wal.db"), true)
            .await
            .expect("connect and migrate (wal)");
        let (mode, timeout_ms) = pragmas(&dbc).await;
        assert_eq!(mode, "wal");
        assert!(
            timeout_ms > 0,
            "busy_timeout must be nonzero, got {timeout_ms}ms"
        );
        let _ = dbc.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Delete profile actively converts a WAL file back. WAL is sticky —
    /// an implicit open inherits it — so the server's `--db-no-wal` escape
    /// hatch must pin DELETE explicitly to actually take effect on an
    /// existing DB.
    #[tokio::test]
    async fn delete_profile_converts_a_wal_file_back() {
        let dir = temp_dir("wal_revert");
        let path = dir.join("revert.db");

        let dbc = connect_and_migrate_wal(&path, true)
            .await
            .expect("wal open");
        assert_eq!(pragmas(&dbc).await.0, "wal");
        let _ = dbc.close().await;

        // Implicit open (no journal pragma) inherits WAL — the stickiness.
        let dbc = connect_and_migrate(&path, false)
            .await
            .expect("implicit open");
        assert_eq!(pragmas(&dbc).await.0, "wal", "implicit open inherits WAL");
        let _ = dbc.close().await;

        let dbc = connect_and_migrate_with(&path, false, JournalMode::Delete)
            .await
            .expect("delete open");
        assert_eq!(
            pragmas(&dbc).await.0,
            "delete",
            "explicit Delete converts back"
        );
        let _ = dbc.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pin the DEFAULT profile's reality so a behavior change in sqlx (which
    /// deliberately leaves journal_mode unset today) or in our code surfaces
    /// here loudly instead of as production surprises: fresh files run
    /// SQLite's `delete` journal with sqlx's 5s busy_timeout.
    #[tokio::test]
    async fn default_profile_is_delete_journal_with_5s_busy_timeout() {
        let dir = temp_dir("default_profile");
        let dbc = connect_and_migrate(&dir.join("default.db"), true)
            .await
            .expect("connect and migrate");
        let (mode, timeout_ms) = pragmas(&dbc).await;
        assert_eq!(mode, "delete");
        assert_eq!(timeout_ms, 5000);
        let _ = dbc.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
